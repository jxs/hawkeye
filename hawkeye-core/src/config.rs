use lazy_static::lazy_static;

// Environment variable names
const HAWKEYE_ENV_ENV: &str = "HAWKEYE_ENV";
const SENTRY_DSN_ENV: &str = "HAWKEYE_SENTRY_DSN";
const SENTRY_ENABLED_ENV: &str = "HAWKEYE_SENTRY_ENABLED";
const SLATE_URL_FILE_EXTS_ENV: &str = "HAWKEYE_SLATE_URL_FILE_EXTS";
const SLATE_URL_SCHEMES_ENV: &str = "HAWKEYE_SLATE_URL_SCHEMES";

lazy_static! {
    // The environment to base logic decisions off of.
    // ie, if local, don't enable sentry and allow more loose validation
    pub static ref HAWKEYE_ENV: String =
        std::env::var(HAWKEYE_ENV_ENV).unwrap_or_else(|_| "local".into());
    // Sentry URL to send events to. Available on the Sentry Project page.
    pub static ref SENTRY_DSN: String = std::env::var(SENTRY_DSN_ENV).unwrap_or_else(|_| "".into());
    pub static ref SENTRY_ENABLED: bool = std::env::var(SENTRY_ENABLED_ENV).unwrap_or_else(|_| "".into()) == "1";
    pub static ref SLATE_URL_FILE_EXTENSIONS: Vec<String> = std::env::var(SLATE_URL_FILE_EXTS_ENV)
        .unwrap_or_else(|_| "jpg,jpeg,png".into())
        .split(',')
        .map(|a| a.trim().to_string())
        .collect();
    pub static ref SLATE_URL_SCHEMES: Vec<String> = std::env::var(SLATE_URL_SCHEMES_ENV)
        .unwrap_or_else(|_| {
            match HAWKEYE_ENV.as_str() {
                "prod" => "http,https",
                _ => "http,https,file",
            }
            .to_string()
        })
        .split(',')
        .map(|a| a.trim().to_string())
        .collect();
}
