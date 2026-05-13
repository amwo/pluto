use std::{pin::Pin, time::Duration};

use anyhow::{Context, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use futures_util::{Stream, StreamExt};
use tonic::{
    Request, Status,
    metadata::AsciiMetadataValue,
    service::{Interceptor, interceptor::InterceptedService},
    transport::{Channel, ClientTlsConfig},
};
use tracing::info;
use yellowstone_grpc_proto::prelude::geyser_client::GeyserClient;

use super::Endpoint;
use super::proto;
use crate::domain::{Commitment, StreamEvent, Subscription};

type Inner = GeyserClient<InterceptedService<Channel, BasicAuth>>;
pub(super) type RawStream = Pin<Box<dyn Stream<Item = Result<StreamEvent>> + Send>>;

pub(super) struct Client {
    inner: Inner,
}

impl Client {
    pub(super) async fn connect(endpoint: &Endpoint) -> Result<Self> {
        let channel = build_endpoint(&endpoint.url)?
            .connect()
            .await
            .context("gRPC connect")?;
        info!(url = %endpoint.url, "grpc connected");
        let auth = BasicAuth::new(&endpoint.username, &endpoint.password)?;
        let inner = GeyserClient::with_interceptor(channel, auth)
            .max_decoding_message_size(64 * 1024 * 1024);
        Ok(Self { inner })
    }

    pub(super) async fn subscribe(
        &mut self,
        subscriptions: &[Subscription],
        commitment: Commitment,
    ) -> Result<RawStream> {
        let request = proto::build_request(subscriptions, commitment);
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        tx.send(request).await?;
        let in_stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        let response = self.inner.subscribe(in_stream).await?;
        info!(?subscriptions, "subscribed");
        let mapped = response.into_inner().filter_map(|msg| {
            let item = match msg {
                Ok(update) => proto::map_update(update),
                Err(status) => Some(Err(status.into())),
            };
            std::future::ready(item)
        });
        Ok(Box::pin(mapped))
    }
}

#[derive(Clone)]
struct BasicAuth(AsciiMetadataValue);

impl BasicAuth {
    fn new(user: &str, pass: &str) -> Result<Self> {
        let raw = format!("Basic {}", B64.encode(format!("{user}:{pass}")));
        Ok(Self(AsciiMetadataValue::try_from(raw)?))
    }
}

impl Interceptor for BasicAuth {
    fn call(&mut self, mut req: Request<()>) -> Result<Request<()>, Status> {
        req.metadata_mut().insert("authorization", self.0.clone());
        Ok(req)
    }
}

fn build_endpoint(url: &str) -> Result<tonic::transport::Endpoint> {
    let url = if url.starts_with("http") {
        url.to_string()
    } else {
        format!("https://{url}")
    };
    let ep = tonic::transport::Endpoint::from_shared(url)?
        .tls_config(ClientTlsConfig::new().with_native_roots())?
        .tcp_nodelay(true)
        .tcp_keepalive(Some(Duration::from_secs(20)))
        .http2_keep_alive_interval(Duration::from_secs(10))
        .keep_alive_timeout(Duration::from_secs(5))
        .keep_alive_while_idle(true)
        .http2_adaptive_window(true)
        .initial_stream_window_size(8 * 1024 * 1024)
        .initial_connection_window_size(16 * 1024 * 1024)
        .buffer_size(64 * 1024)
        .connect_timeout(Duration::from_secs(5));
    Ok(ep)
}
