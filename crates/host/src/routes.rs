//! AGENTS.md routing rules parser (M7 helper).

use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct AgentsRoutingRules {
    pub default_agent: Option<String>,
    pub channel_pins: HashMap<String, String>,
    pub peer_pins: HashMap<(String, String), String>,
}

/// Parse a simple Markdown routing spec:
///
/// ```markdown
/// ## Defaults
/// default -> alpha
///
/// ## Channel pins
/// cli -> alpha
/// discord -> beta
///
/// ## Peer pins
/// cli:matt -> alpha
/// ```
pub fn parse_agents_md(text: &str) -> AgentsRoutingRules {
    let mut rules = AgentsRoutingRules::default();
    let mut section = String::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("## ") {
            section = rest.trim().to_lowercase();
            continue;
        }
        let (lhs, rhs) = if let Some(idx) = line.find('\u{2192}') {
            (line[..idx].trim().to_string(), line[idx + "→".len()..].trim().to_string())
        } else if let Some(idx) = line.find("->") {
            (line[..idx].trim().to_string(), line[idx + 2..].trim().to_string())
        } else {
            continue;
        };
        if rhs.is_empty() {
            continue;
        }
        match section.as_str() {
            "defaults" => {
                rules.default_agent = Some(rhs);
            }
            "channel pins" => {
                rules.channel_pins.insert(lhs, rhs);
            }
            "peer pins" => {
                if let Some((c, p)) = lhs.split_once(':') {
                    rules.peer_pins.insert((c.trim().to_string(), p.trim().to_string()), rhs);
                }
            }
            _ => {}
        }
    }
    rules
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sections() {
        let body = "
## Defaults
default -> alpha

## Channel pins
cli -> alpha
discord -> beta

## Peer pins
cli:matt -> support
        ";
        let r = parse_agents_md(body);
        assert_eq!(r.default_agent.as_deref(), Some("alpha"));
        assert_eq!(r.channel_pins.get("discord").map(|s| s.as_str()), Some("beta"));
        assert_eq!(
            r.peer_pins.get(&("cli".to_string(), "matt".to_string())).map(|s| s.as_str()),
            Some("support")
        );
    }

    #[test]
    fn handles_unicode_arrow() {
        let body = "## Defaults\ndefault → alpha\n";
        let r = parse_agents_md(body);
        assert_eq!(r.default_agent.as_deref(), Some("alpha"));
    }
}
