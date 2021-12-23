use lazy_static::lazy_static;

// Environment variable names
const SLATE_URL_FILE_EXTS_ENV: &str = "HAWKEYE_SLATE_URL_FILE_EXTS";
const SLATE_URL_SCHEMES_ENV: &str = "HAWKEYE_SLATE_URL_SCHEMES";
const HAWKEYE_ENV_ENV: &str = "HAWKEYE_ENV";

lazy_static! {
    pub static ref HAWKEYE_ENV: String = std::env::var(HAWKEYE_ENV_ENV).unwrap_or("local".into());
    pub static ref SLATE_URL_FILE_EXTENSIONS: Vec<String> = std::env::var(SLATE_URL_FILE_EXTS_ENV)
        .unwrap_or("jpg,jpeg,png".into())
        .split(",")
        .map(|a| a.trim().to_string())
        .collect();
    pub static ref SLATE_URL_SCHEMES: Vec<String> = std::env::var(SLATE_URL_SCHEMES_ENV)
        .unwrap_or_else(|_| {
            match HAWKEYE_ENV.to_string().as_str() {
                "prod" => "http,https".to_string(),
                _ => "http,https,file".to_string(),
            }
        })
        .split(",")
        .map(|a| a.trim().to_string())
        .collect();
}
