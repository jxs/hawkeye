use crate::config;
use sentry::ClientInitGuard;
use std::borrow::Cow;

/// Helper for bootstrapping Sentry based on HAWKEYE_ENV to capture panics and logs for context.
pub fn maybe_bootstrap_sentry() -> Option<ClientInitGuard> {
    if !*config::SENTRY_ENABLED {
        log::debug!("SENTRY_ENABLED is not true. Skipping Sentry initialization.");
        return None;
    }

    // Since Sentry is enabled, sanity check the URL.
    if config::SENTRY_DSN.len() < 10 || !config::SENTRY_DSN.starts_with("https://") {
        log::error!("Invalid SENTRY_DSN supplied when SENTRY_ENABLED=true. Skipping Sentry initialization...");
        return None;
    }

    let mut log_builder = pretty_env_logger::formatted_builder();
    log_builder.parse_filters("info");
    let logger = sentry_log::SentryLogger::with_dest(log_builder.build());
    log::set_boxed_logger(Box::new(logger)).unwrap();
    // Log <= INFO as breadcrumbs. Anything higher is an "error" which generates a Sentry Issue.
    log::set_max_level(log::LevelFilter::Info);

    // The caller should keep this reference alive (ie, in scope) or Sentry mechanics will not work.
    let sentry_client = sentry::init((
        config::SENTRY_DSN.as_str(),
        sentry::apply_defaults(sentry::ClientOptions {
            release: sentry::release_name!(),
            debug: false, // see what Sentry is doing to debug config issues
            environment: Some(Cow::from(config::HAWKEYE_ENV.as_str())),
            ..Default::default()
        }),
    ));

    Some(sentry_client)
}

#[cfg(test)]
mod tests {
    use crate::utils;
    use std::env;

    #[test]
    fn test_sentry_bootstrapping() {
        // not enabled
        env::set_var("HAWKEYE_SENTRY_DSN", "https://abc123");
        env::set_var("HAWKEYE_SENTRY_ENABLED", "0");
        let sentry = utils::maybe_bootstrap_sentry();
        assert!(sentry.is_none());

        // missing DSN
        env::remove_var("HAWKEYE_SENTRY_DSN");
        env::set_var("HAWKEYE_SENTRY_ENABLED", "1");
        let sentry = utils::maybe_bootstrap_sentry();
        assert!(sentry.is_none());

        // invalid DSN
        env::set_var("HAWKEYE_SENTRY_DSN", "oops");
        env::set_var("HAWKEYE_SENTRY_ENABLED", "1");
        let sentry = utils::maybe_bootstrap_sentry();
        assert!(sentry.is_none());
    }
}
