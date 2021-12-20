use lazy_static::lazy_static;

// Environment variable names
const SLATE_URL_FILE_EXTS_ENV: &str = "HAWKEYE_SLATE_URL_FILE_EXTS";
const SLATE_URL_SCHEMES_ENV: &str = "HAWKEYE_SLATE_URL_SCHEMES";


lazy_static! {
    pub static ref SLATE_URL_FILE_EXTENSIONS: Vec<String> =
        std::env::var(SLATE_URL_FILE_EXTS_ENV)
            .unwrap_or("jpg,jpeg,png".into())
            .split(",")
            .map(|a| a.trim().to_string())
            .collect();

    pub static ref SLATE_URL_SCHEMES: Vec<String> =
        std::env::var(SLATE_URL_SCHEMES_ENV)
            .unwrap_or("http,https".into())
            .split(",")
            .map(|a| a.trim().to_string())
            .collect();
}
