//! Server configuration parsed from the command line.

use deepcore_shared::protocol::DEFAULT_PORT;

/// Everything the server needs to start.
#[derive(Debug, Clone, bevy::prelude::Resource)]
pub struct ServerConfig {
    /// UDP port to bind.
    pub port: u16,
    /// netcode protocol id, shared with the client.
    pub protocol_id: u64,
    /// World seed.
    pub seed: u64,
    /// Number of headless bot clients to run for load testing.
    pub bots: usize,
    /// Simulated one-way latency in milliseconds, applied to the transport.
    pub simulate_latency_ms: u32,
    /// Simulated packet loss, as a percentage.
    pub simulate_loss_percent: u32,
    /// Run forever instead of exiting (used by tests with a tick budget).
    pub max_ticks: Option<u64>,
    /// Content directory to load, overriding the default `assets/data`.
    pub data_dir: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: DEFAULT_PORT,
            protocol_id: 0x0DE9_C0DE,
            seed: 2024,
            bots: 0,
            simulate_latency_ms: 0,
            simulate_loss_percent: 0,
            max_ticks: None,
            data_dir: None,
        }
    }
}

/// Parses `--flag value` style arguments.
///
/// Unknown arguments are reported rather than ignored, so a typo in a launch
/// script does not silently run with defaults.
pub fn parse_args(args: &[String]) -> Result<ServerConfig, String> {
    let mut config = ServerConfig::default();
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        let mut take_value = || -> Result<String, String> {
            index += 1;
            args.get(index)
                .cloned()
                .ok_or_else(|| format!("{flag} needs a value"))
        };
        match flag {
            "--port" => {
                let value = take_value()?;
                config.port = value
                    .parse()
                    .map_err(|_| format!("--port: {value:?} is not a valid port"))?;
            }
            "--seed" => {
                let value = take_value()?;
                config.seed = value
                    .parse()
                    .map_err(|_| format!("--seed: {value:?} is not a number"))?;
            }
            "--bots" => {
                let value = take_value()?;
                config.bots = value
                    .parse()
                    .map_err(|_| format!("--bots: {value:?} is not a number"))?;
            }
            "--simulate-latency" => {
                let value = take_value()?;
                config.simulate_latency_ms = value
                    .parse()
                    .map_err(|_| format!("--simulate-latency: {value:?} is not a number"))?;
            }
            "--simulate-loss" => {
                let value = take_value()?;
                config.simulate_loss_percent = value
                    .parse()
                    .map_err(|_| format!("--simulate-loss: {value:?} is not a number"))?;
                if config.simulate_loss_percent > 100 {
                    return Err("--simulate-loss must be between 0 and 100".to_string());
                }
            }
            "--max-ticks" => {
                let value = take_value()?;
                config.max_ticks = Some(
                    value
                        .parse()
                        .map_err(|_| format!("--max-ticks: {value:?} is not a number"))?,
                );
            }
            "--data" => {
                config.data_dir = Some(take_value()?);
            }
            "-h" | "--help" => {
                return Err(help_text().to_string());
            }
            other => return Err(format!("unknown argument {other:?}\n\n{}", help_text())),
        }
        index += 1;
    }
    Ok(config)
}

/// Usage text.
pub fn help_text() -> &'static str {
    "Deepcore server\n\n\
     usage: server [options]\n\n\
     options:\n\
       --port <n>                UDP port to listen on (default 43210)\n\
       --seed <n>                world seed (default 2024)\n\
       --bots <n>                run n headless bot clients for load testing\n\
       --simulate-latency <ms>   add one-way latency to all traffic\n\
       --simulate-loss <pct>     drop pct% of packets\n\
       --max-ticks <n>           stop after n simulation ticks\n\
       --data <dir>              load content from <dir> instead of assets/data\n\
       -h, --help                show this help"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<ServerConfig, String> {
        parse_args(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn defaults_are_sensible() {
        let config = parse(&[]).expect("no args is valid");
        assert_eq!(config.port, DEFAULT_PORT);
        assert_eq!(config.bots, 0);
        assert_eq!(config.simulate_latency_ms, 0);
    }

    #[test]
    fn flags_are_parsed() {
        let config = parse(&[
            "--port",
            "5000",
            "--seed",
            "77",
            "--bots",
            "8",
            "--simulate-latency",
            "150",
            "--simulate-loss",
            "5",
        ])
        .expect("valid");
        assert_eq!(config.port, 5000);
        assert_eq!(config.seed, 77);
        assert_eq!(config.bots, 8);
        assert_eq!(config.simulate_latency_ms, 150);
        assert_eq!(config.simulate_loss_percent, 5);
    }

    #[test]
    fn bad_values_are_reported_not_ignored() {
        assert!(parse(&["--port", "notaport"]).is_err());
        assert!(
            parse(&["--port"]).is_err(),
            "a flag with no value must fail"
        );
        assert!(parse(&["--bogus"]).is_err(), "unknown flags must fail");
        assert!(
            parse(&["--simulate-loss", "150"]).is_err(),
            "loss above 100%"
        );
    }

    #[test]
    fn help_is_reachable() {
        let err = parse(&["--help"]).expect_err("help is not a config");
        assert!(err.contains("usage: server"));
    }
}
