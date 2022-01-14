use lazy_static::lazy_static;

const HAWKEYE_ENV_ENV: &str = "HAWKEYE_ENV";
const SENTRY_DSN_ENV: &str = "HAWKEYE_SENTRY_DSN";
const SENTRY_ENABLED_ENV: &str = "HAWKEYE_SENTRY_ENABLED";

lazy_static! {
    // The environment to base logic decisions off of.
    // ie, if local, don't enable sentry and allow more loose validation
    pub static ref HAWKEYE_ENV: String = std::env::var(HAWKEYE_ENV_ENV).unwrap_or_else(|_| "local".into());

    // Sentry URL to send events to. Available on the Sentry Project page.
    pub static ref SENTRY_DSN: String = std::env::var(SENTRY_DSN_ENV).unwrap_or_else(|_| "".into());
    pub static ref SENTRY_ENABLED: bool = std::env::var(SENTRY_ENABLED_ENV).unwrap_or_else(|_| "".into()) == "1";
}
