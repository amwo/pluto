#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[allow(dead_code)]
pub enum SkipReason {
    TooLarge,
    TooSmall,
    UnknownRoute,
    StaleDetection,
    ExistingPosition,
    HighPriceImpact,
    RiskLimit,
    DecodeUncertain,
    RateLimited,
}
