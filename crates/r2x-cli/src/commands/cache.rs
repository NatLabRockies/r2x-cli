use crate::config_manager::Config;
use crate::logger;
use crate::GlobalOpts;
use clap::Subcommand;
use std::fs;
use std::path::PathBuf;

pub fn handle_cache(action: CacheAction, opts: GlobalOpts) {
    match action {
        CacheAction::Clean => {
            clean_cache(opts);
        }
        CacheAction::Path { new_path } => {
            handle_cache_path(new_path, opts);
        }
    }
}

fn clean_cache(_opts: GlobalOpts) {
    match Config::load() {
        Ok(config) => {
            let cache_path = config.get_cache_path();
            let cache_dir = PathBuf::from(&cache_path);

            if !cache_dir.exists() {
                logger::debug("Cache folder already clean");
                return;
            }

            match fs::remove_dir_all(&cache_dir) {
                Ok(_) => {
                    logger::success("Cache folder cleaned");
                }
                Err(e) => {
                    logger::error(&format!("Failed to clean cache folder: {}", e));
                }
            }
        }
        Err(e) => {
            logger::error(&format!("Failed to load config: {}", e));
        }
    }
}

fn handle_cache_path(new_path: Option<String>, _opts: GlobalOpts) {
    match Config::load() {
        Ok(mut config) => {
            if let Some(path) = new_path {
                let cache_path = PathBuf::from(&path);

                if let Err(e) = fs::create_dir_all(&cache_path) {
                    logger::error(&format!("Failed to create cache directory: {}", e));
                    return;
                }

                config.cache_path = Some(path.clone());
                if let Err(e) = config.save() {
                    logger::error(&format!("Failed to save config: {}", e));
                    return;
                }

                logger::success(&format!("Cache path set to {}", path));
            } else {
                let cache_path = config.get_cache_path();
                println!("{}", cache_path);
            }
        }
        Err(e) => {
            logger::error(&format!("Failed to load config: {}", e));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn normal_opts() -> GlobalOpts {
        GlobalOpts {
            quiet: false,
            verbose: 0,
            log_python: false,
        }
    }

    fn verbose_opts() -> GlobalOpts {
        GlobalOpts {
            quiet: false,
            verbose: 1,
            log_python: false,
        }
    }

    #[test]
    fn test_cache_clean() {
        handle_cache(CacheAction::Clean, normal_opts());
    }

    #[test]
    fn test_cache_clean_verbose() {
        handle_cache(CacheAction::Clean, verbose_opts());
    }

    #[test]
    fn test_cache_path() {
        handle_cache(CacheAction::Path { new_path: None }, normal_opts());
    }

    #[test]
    fn test_cache_path_verbose() {
        handle_cache(CacheAction::Path { new_path: None }, verbose_opts());
    }

    #[test]
    fn test_cache_path_set() {
        handle_cache(
            CacheAction::Path {
                new_path: Some("/tmp/r2x-cache".to_string()),
            },
            normal_opts(),
        );
    }

    #[test]
    fn test_cache_path_set_verbose() {
        handle_cache(
            CacheAction::Path {
                new_path: Some("/tmp/r2x-cache-test".to_string()),
            },
            verbose_opts(),
        );
    }
}
#[derive(Subcommand, Debug, Clone)]
pub enum CacheAction {
    /// Clean the cache folder
    Clean,
    /// Get or set cache path
    Path {
        /// Optional new cache path to set
        new_path: Option<String>,
    },
}
