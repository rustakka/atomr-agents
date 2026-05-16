//! `HostConfig` — the deserialized `<root>/config.yaml`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::{HostError, HostResult};
use crate::layout::{default_root, HostPaths};

/// A single inference provider entry in `config.yaml`.
///
/// The host doesn't open API keys directly — it surfaces the env-var
/// name so callers can resolve the value just-in-time and keep secrets
/// off disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub name: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub api_key_env: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct HostConfig {
    pub paths: HostPaths,
    pub version: u32,
    pub default_agent: Option<String>,
    pub default_model: Option<String>,
    pub providers: BTreeMap<String, ProviderConfig>,
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl HostConfig {
    /// Empty config rooted at `paths`. Returned when `config.yaml` is missing.
    pub fn empty(paths: HostPaths) -> Self {
        Self {
            paths,
            version: 1,
            default_agent: None,
            default_model: None,
            providers: BTreeMap::new(),
            extra: BTreeMap::new(),
        }
    }

    pub fn load_default() -> HostResult<Self> {
        Self::load(default_root())
    }

    pub fn load(root: impl Into<std::path::PathBuf>) -> HostResult<Self> {
        let paths = HostPaths::new(root);
        let cfg_path = paths.config_yaml();
        if !cfg_path.is_file() {
            return Ok(Self::empty(paths));
        }
        let text = std::fs::read_to_string(&cfg_path)
            .map_err(|e| HostError::io(&cfg_path, e))?;
        let raw: serde_yaml::Value = serde_yaml::from_str(&text)
            .map_err(|e| HostError::yaml(&cfg_path, e))?;
        Self::from_yaml_value(raw, paths)
    }

    pub fn from_yaml_value(raw: serde_yaml::Value, paths: HostPaths) -> HostResult<Self> {
        let map = match raw {
            serde_yaml::Value::Mapping(m) => m,
            serde_yaml::Value::Null => return Ok(Self::empty(paths)),
            _ => {
                return Err(HostError::config(format!(
                    "{}: top-level must be a YAML mapping",
                    paths.config_yaml().display()
                )))
            }
        };

        let mut cfg = Self::empty(paths);

        for (k, v) in map {
            let key = match k.as_str() {
                Some(s) => s.to_string(),
                None => continue,
            };
            match key.as_str() {
                "version" => {
                    if let Some(n) = v.as_u64() {
                        cfg.version = n as u32;
                    }
                }
                "default_agent" => {
                    cfg.default_agent = v.as_str().map(|s| s.to_string());
                }
                "default_model" => {
                    cfg.default_model = v.as_str().map(|s| s.to_string());
                }
                "providers" => {
                    let providers_map = v.as_mapping().ok_or_else(|| {
                        HostError::config("`providers` must be a mapping of name → provider config")
                    })?;
                    for (pk, pv) in providers_map {
                        let pname = pk
                            .as_str()
                            .ok_or_else(|| HostError::config("provider key must be a string"))?
                            .to_string();
                        let pmap = pv.as_mapping().ok_or_else(|| {
                            HostError::config(format!("provider `{pname}` must be a mapping"))
                        })?;
                        let kind = pmap
                            .get(serde_yaml::Value::String("kind".to_string()))
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| {
                                HostError::config(format!(
                                    "provider `{pname}` is missing a string `kind`"
                                ))
                            })?
                            .to_string();
                        let api_key_env = pmap
                            .get(serde_yaml::Value::String("api_key_env".to_string()))
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        let base_url = pmap
                            .get(serde_yaml::Value::String("base_url".to_string()))
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        let mut extra = BTreeMap::new();
                        for (xk, xv) in pmap {
                            let xs = match xk.as_str() {
                                Some(s) => s,
                                None => continue,
                            };
                            if matches!(xs, "kind" | "api_key_env" | "base_url") {
                                continue;
                            }
                            extra.insert(xs.to_string(), yaml_to_json(xv.clone()));
                        }
                        cfg.providers.insert(
                            pname.clone(),
                            ProviderConfig {
                                name: pname,
                                kind,
                                api_key_env,
                                base_url,
                                extra,
                            },
                        );
                    }
                }
                _ => {
                    cfg.extra.insert(key, yaml_to_json(v));
                }
            }
        }
        Ok(cfg)
    }

    /// Render back to YAML — used by `atomr-host init`.
    pub fn to_yaml(&self) -> serde_yaml::Value {
        let mut out = serde_yaml::Mapping::new();
        out.insert(
            serde_yaml::Value::String("version".into()),
            serde_yaml::Value::Number((self.version as u64).into()),
        );
        if let Some(a) = &self.default_agent {
            out.insert(
                serde_yaml::Value::String("default_agent".into()),
                serde_yaml::Value::String(a.clone()),
            );
        }
        if let Some(m) = &self.default_model {
            out.insert(
                serde_yaml::Value::String("default_model".into()),
                serde_yaml::Value::String(m.clone()),
            );
        }
        if !self.providers.is_empty() {
            let mut pm = serde_yaml::Mapping::new();
            for (name, p) in &self.providers {
                let mut entry = serde_yaml::Mapping::new();
                entry.insert(
                    serde_yaml::Value::String("kind".into()),
                    serde_yaml::Value::String(p.kind.clone()),
                );
                if let Some(env) = &p.api_key_env {
                    entry.insert(
                        serde_yaml::Value::String("api_key_env".into()),
                        serde_yaml::Value::String(env.clone()),
                    );
                }
                if let Some(url) = &p.base_url {
                    entry.insert(
                        serde_yaml::Value::String("base_url".into()),
                        serde_yaml::Value::String(url.clone()),
                    );
                }
                for (k, v) in &p.extra {
                    entry.insert(
                        serde_yaml::Value::String(k.clone()),
                        json_to_yaml(v.clone()),
                    );
                }
                pm.insert(serde_yaml::Value::String(name.clone()), serde_yaml::Value::Mapping(entry));
            }
            out.insert(
                serde_yaml::Value::String("providers".into()),
                serde_yaml::Value::Mapping(pm),
            );
        }
        for (k, v) in &self.extra {
            out.insert(serde_yaml::Value::String(k.clone()), json_to_yaml(v.clone()));
        }
        serde_yaml::Value::Mapping(out)
    }

    pub fn to_yaml_string(&self) -> HostResult<String> {
        serde_yaml::to_string(&self.to_yaml())
            .map_err(|e| HostError::yaml(self.paths.config_yaml(), e))
    }
}

pub(crate) fn yaml_to_json(v: serde_yaml::Value) -> serde_json::Value {
    match v {
        serde_yaml::Value::Null => serde_json::Value::Null,
        serde_yaml::Value::Bool(b) => serde_json::Value::Bool(b),
        serde_yaml::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                serde_json::Value::Number(i.into())
            } else if let Some(u) = n.as_u64() {
                serde_json::Value::Number(u.into())
            } else if let Some(f) = n.as_f64() {
                serde_json::Number::from_f64(f)
                    .map(serde_json::Value::Number)
                    .unwrap_or(serde_json::Value::Null)
            } else {
                serde_json::Value::Null
            }
        }
        serde_yaml::Value::String(s) => serde_json::Value::String(s),
        serde_yaml::Value::Sequence(s) => {
            serde_json::Value::Array(s.into_iter().map(yaml_to_json).collect())
        }
        serde_yaml::Value::Mapping(m) => {
            let mut o = serde_json::Map::new();
            for (k, v) in m {
                let ks = match k {
                    serde_yaml::Value::String(s) => s,
                    other => match serde_yaml::to_string(&other) {
                        Ok(s) => s.trim().to_string(),
                        Err(_) => continue,
                    },
                };
                o.insert(ks, yaml_to_json(v));
            }
            serde_json::Value::Object(o)
        }
        serde_yaml::Value::Tagged(t) => yaml_to_json(t.value),
    }
}

pub(crate) fn json_to_yaml(v: serde_json::Value) -> serde_yaml::Value {
    match v {
        serde_json::Value::Null => serde_yaml::Value::Null,
        serde_json::Value::Bool(b) => serde_yaml::Value::Bool(b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                serde_yaml::Value::Number(i.into())
            } else if let Some(u) = n.as_u64() {
                serde_yaml::Value::Number(u.into())
            } else if let Some(f) = n.as_f64() {
                serde_yaml::Value::Number(f.into())
            } else {
                serde_yaml::Value::Null
            }
        }
        serde_json::Value::String(s) => serde_yaml::Value::String(s),
        serde_json::Value::Array(a) => {
            serde_yaml::Value::Sequence(a.into_iter().map(json_to_yaml).collect())
        }
        serde_json::Value::Object(o) => {
            let mut m = serde_yaml::Mapping::new();
            for (k, v) in o {
                m.insert(serde_yaml::Value::String(k), json_to_yaml(v));
            }
            serde_yaml::Value::Mapping(m)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn empty_when_no_file() {
        let tmp = tempdir().unwrap();
        let cfg = HostConfig::load(tmp.path()).unwrap();
        assert_eq!(cfg.version, 1);
        assert!(cfg.providers.is_empty());
        assert_eq!(cfg.paths.root, tmp.path());
    }

    #[test]
    fn parses_full_config() {
        let tmp = tempdir().unwrap();
        let body = r#"
version: 1
default_agent: alpha
default_model: gpt-4o
providers:
  openai:
    kind: openai
    api_key_env: OPENAI_API_KEY
  anthropic:
    kind: anthropic
    api_key_env: ANTHROPIC_API_KEY
    base_url: https://example.com/v1
extra:
  trace: true
"#;
        std::fs::write(tmp.path().join("config.yaml"), body).unwrap();
        let cfg = HostConfig::load(tmp.path()).unwrap();
        assert_eq!(cfg.default_agent.as_deref(), Some("alpha"));
        assert_eq!(cfg.default_model.as_deref(), Some("gpt-4o"));
        assert_eq!(cfg.providers.len(), 2);
        let a = &cfg.providers["anthropic"];
        assert_eq!(a.kind, "anthropic");
        assert_eq!(a.base_url.as_deref(), Some("https://example.com/v1"));
        assert!(cfg.extra.contains_key("extra"));
    }
}
