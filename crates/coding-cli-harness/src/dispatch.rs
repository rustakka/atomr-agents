//! `Callable` glue: parse a Value into a `CliRequest`, serialize a
//! `CliResult` back out.

use atomr_agents_core::{AgentError, Value};

use atomr_agents_coding_cli_core::{CliRequest, CliResult};

pub(crate) fn parse_request(input: Value) -> Result<CliRequest, AgentError> {
    serde_json::from_value::<CliRequest>(input).map_err(AgentError::from)
}

pub(crate) fn encode_result(result: &CliResult) -> Result<Value, AgentError> {
    serde_json::to_value(result).map_err(AgentError::from)
}
