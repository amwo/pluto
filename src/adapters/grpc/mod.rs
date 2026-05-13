use std::{collections::HashMap, future, pin::Pin, time::Duration};

use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use futures_util::{Stream, StreamExt};
use tokio::sync::mpsc;
use tonic::{
    Request, Status,
    metadata::AsciiMetadataValue,
    service::{Interceptor, interceptor::InterceptedService},
    transport::{Channel, ClientTlsConfig, Endpoint},
};
use tracing::{info, warn};
use yellowstone_grpc_proto::prelude::{
    CommitmentLevel, SubscribeRequest, SubscribeRequestFilterTransactions,
    geyser_client::GeyserClient, subscribe_update::UpdateOneof,
};

use crate::domain::{Commitment, Pubkey, Signature, Slot, StreamEvent, Subscription};

type Client = GeyserClient<InterceptedService<Channel, BasicAuth>>;
type RawStream = Pin<Box<dyn Stream<Item = Result<StreamEvent>> + Send>>;

#[derive(Clone, Debug)]
pub struct GrpcEndpoint {
    pub url: String,
    pub username: String,
    pub password: String,
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

fn commitment_to_proto(c: Commitment) -> i32 {
    match c {
        Commitment::Processed => CommitmentLevel::Processed as i32,
        Commitment::Confirmed => CommitmentLevel::Confirmed as i32,
        Commitment::Finalized => CommitmentLevel::Finalized as i32,
    }
}

pub fn spawn_stream(
    endpoint: GrpcEndpoint,
    subscriptions: Vec<Subscription>,
    commitment: Commitment,
) -> mpsc::Receiver<StreamEvent> {
    let (tx, rx) = mpsc::channel(256);
    tokio::spawn(async move {
        let mut delay = Duration::from_secs(1);
        loop {
            let started = std::time::Instant::now();
            match run(&endpoint, &subscriptions, commitment, &tx).await {
                Ok(()) => return,
                Err(e) => warn!(error = %e, "grpc stream lost"),
            }
            if tx.is_closed() {
                return;
            }
            if started.elapsed() > Duration::from_secs(60) {
                delay = Duration::from_secs(1);
            }
            warn!(delay_secs = delay.as_secs(), "reconnecting");
            tokio::time::sleep(delay).await;
            delay = (delay * 2).min(Duration::from_secs(60));
        }
    });
    rx
}

async fn run(
    endpoint: &GrpcEndpoint,
    subscriptions: &[Subscription],
    commitment: Commitment,
    tx: &mpsc::Sender<StreamEvent>,
) -> Result<()> {
    let mut client = connect(endpoint).await?;
    let mut updates = subscribe(&mut client, subscriptions, commitment).await?;
    while let Some(msg) = updates.next().await {
        let update = msg.context("grpc stream error")?;
        if tx.send(update).await.is_err() {
            return Ok(());
        }
    }
    anyhow::bail!("grpc stream ended")
}

async fn connect(endpoint: &GrpcEndpoint) -> Result<Client> {
    let channel = build_endpoint(&endpoint.url)?
        .connect()
        .await
        .context("gRPC connect")?;
    info!(url = %endpoint.url, "grpc connected");
    let auth = BasicAuth::new(&endpoint.username, &endpoint.password)?;
    Ok(GeyserClient::with_interceptor(channel, auth).max_decoding_message_size(64 * 1024 * 1024))
}

fn build_endpoint(url: &str) -> Result<Endpoint> {
    let url = if url.starts_with("http") {
        url.to_string()
    } else {
        format!("https://{url}")
    };

    let ep = Endpoint::from_shared(url)?
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

async fn subscribe(
    client: &mut Client,
    subscriptions: &[Subscription],
    commitment: Commitment,
) -> Result<RawStream> {
    let request = build_request(subscriptions, commitment);

    let (tx, rx) = tokio::sync::mpsc::channel(8);
    tx.send(request).await?;
    let in_stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    let response = client.subscribe(in_stream).await?;
    info!(?subscriptions, "subscribed");
    let raw = response.into_inner();

    let mapped = raw.filter_map(|msg| {
        let item: Option<Result<StreamEvent>> = match msg {
            Ok(update) => match update.update_oneof {
                Some(UpdateOneof::Transaction(t)) => Some(decode_tx(t)),
                Some(UpdateOneof::Ping(_)) | Some(UpdateOneof::Pong(_)) => {
                    Some(Ok(StreamEvent::Heartbeat))
                }
                Some(_) | None => None,
            },
            Err(status) => Some(Err(status.into())),
        };
        future::ready(item)
    });
    Ok(Box::pin(mapped))
}

fn build_request(subscriptions: &[Subscription], commitment: Commitment) -> SubscribeRequest {
    let mut transactions = HashMap::new();
    for (i, sub) in subscriptions.iter().enumerate() {
        match sub {
            Subscription::WalletTransactions(wallets) => {
                transactions.insert(
                    format!("tx_{i}"),
                    SubscribeRequestFilterTransactions {
                        vote: Some(false),
                        failed: Some(false),
                        signature: None,
                        account_include: wallets.iter().map(Pubkey::to_string).collect(),
                        account_exclude: vec![],
                        account_required: vec![],
                    },
                );
            }
        }
    }
    SubscribeRequest {
        transactions,
        commitment: Some(commitment_to_proto(commitment)),
        ..Default::default()
    }
}

fn decode_tx(
    t: yellowstone_grpc_proto::prelude::SubscribeUpdateTransaction,
) -> Result<StreamEvent> {
    let slot = Slot::from(t.slot);
    let bytes = t
        .transaction
        .ok_or_else(|| anyhow!("tx update missing transaction body"))?
        .signature;
    let signature = Signature::try_from_slice(&bytes)?;
    Ok(StreamEvent::Tx { slot, signature })
}
