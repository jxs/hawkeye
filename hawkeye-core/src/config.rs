use lazy_static::lazy_static;

// Environment variable names
const HAWKEYE_ENV_ENV: &str = "HAWKEYE_ENV";
const SENTRY_DSN_ENV: &str = "HAWKEYE_SENTRY_DSN";
const SENTRY_ENABLED_ENV: &str = "HAWKEYE_SENTRY_ENABLED";

lazy_static! {
    // The environment to base logic decisions off of.
    // ie, if local, don't enable sentry and allow more loose validation
    pub static ref HAWKEYE_ENV: String =
        std::env::var(HAWKEYE_ENV_ENV).unwrap_or_else(|_| "local".to_owned());
    // Sentry URL to send events to. Available on the Sentry Project page.
    pub static ref SENTRY_DSN: String = std::env::var(SENTRY_DSN_ENV).unwrap_or_else(|_| "".to_owned());
    pub static ref SENTRY_ENABLED: bool = std::env::var(SENTRY_ENABLED_ENV).unwrap_or_else(|_| "".to_owned()) == "1";
    pub static ref SLATE_URL_FILE_EXTENSIONS: [String; 3] = [
        "jpg".to_string(), "jpeg".to_string(), "png".to_string()
    ];
    pub static ref SLATE_URL_SCHEMES: Vec<String> = match HAWKEYE_ENV.as_str() {
        "prod" => vec!["http".to_string(), "https".to_string()],
        _ => vec!["http".to_string(), "https".to_string(), "file".to_string()],
    };
}
