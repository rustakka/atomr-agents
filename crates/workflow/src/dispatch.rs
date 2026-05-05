//! Send-API analogue: dynamic fan-out at runtime.
//!
//! `dispatch_fan_out(producer, target, concurrency)` runs `producer`
//! once, expects a JSON array, and dispatches each element through
//! `target` with bounded concurrency. Order is preserved in the
//! returned Vec.

use std::sync::Arc;

use atomr_agents_callable::CallableHandle;
use atomr_agents_core::{AgentError, CallCtx, Result, Value};
use tokio::sync::Semaphore;

/// Run `producer` to obtain a list of inputs, then dispatch each
/// through `target` with at most `concurrency` running concurrently.
/// Returns the per-input outputs in original order.
pub async fn dispatch_fan_out(
    producer: CallableHandle,
    target: CallableHandle,
    concurrency: u32,
    seed_input: Value,
    ctx: CallCtx,
) -> Result<Vec<Value>> {
    let produced = producer.call(seed_input, ctx.clone()).await?;
    let inputs: Vec<Value> = match produced {
        Value::Array(a) => a,
        Value::Null => Vec::new(),
        single => vec![single],
    };
    let sem = Arc::new(Semaphore::new(concurrency.max(1) as usize));
    let mut handles = Vec::with_capacity(inputs.len());
    for (i, inp) in inputs.into_iter().enumerate() {
        let target = target.clone();
        let ctx = ctx.clone();
        let sem = sem.clone();
        handles.push(tokio::spawn(async move {
            let _permit = sem
                .acquire()
                .await
                .map_err(|e| AgentError::Internal(e.to_string()))?;
            let v = target.call(inp, ctx).await?;
            Ok::<_, AgentError>((i, v))
        }));
    }
    let mut out: Vec<(usize, Value)> = Vec::with_capacity(handles.len());
    for h in handles {
        let pair = h.await.map_err(|e| AgentError::Internal(e.to_string()))??;
        out.push(pair);
    }
    out.sort_by_key(|(i, _)| *i);
    Ok(out.into_iter().map(|(_, v)| v).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_callable::FnCallable;
    use atomr_agents_core::{IterationBudget, MoneyBudget, TimeBudget, TokenBudget};
    use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
    use std::time::Duration;

    fn ctx() -> CallCtx {
        CallCtx {
            agent_id: None,
            tokens: TokenBudget::new(1000),
            time: TimeBudget::new(Duration::from_secs(5)),
            money: MoneyBudget::from_usd(0.10),
            iterations: IterationBudget::new(100),
            trace: vec![],
        }
    }

    #[tokio::test]
    async fn fan_out_preserves_order_and_calls_per_input() {
        let producer: CallableHandle = Arc::new(FnCallable::labeled(
            "producer",
            |_v: Value, _ctx| async move { Ok(serde_json::json!([1, 2, 3, 4, 5])) },
        ));
        let calls = Arc::new(AtomicUsize::new(0));
        let calls2 = calls.clone();
        let target: CallableHandle = Arc::new(FnCallable::labeled(
            "target",
            move |v: Value, _ctx| {
                let calls = calls2.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(serde_json::json!(v.as_i64().unwrap() * 10))
                }
            },
        ));
        let out = dispatch_fan_out(producer, target, 2, Value::Null, ctx()).await.unwrap();
        assert_eq!(out.len(), 5);
        assert_eq!(out[0], serde_json::json!(10));
        assert_eq!(out[4], serde_json::json!(50));
        assert_eq!(calls.load(Ordering::SeqCst), 5);
    }

    #[tokio::test]
    async fn fan_out_respects_concurrency() {
        let producer: CallableHandle = Arc::new(FnCallable::labeled(
            "p",
            |_v: Value, _ctx| async move { Ok(serde_json::json!([1, 2, 3, 4, 5, 6, 7, 8])) },
        ));
        let active = Arc::new(AtomicU32::new(0));
        let max_seen = Arc::new(AtomicU32::new(0));
        let active2 = active.clone();
        let max2 = max_seen.clone();
        let target: CallableHandle = Arc::new(FnCallable::labeled(
            "t",
            move |v: Value, _ctx| {
                let active = active2.clone();
                let max_seen = max2.clone();
                async move {
                    let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                    let mut m = max_seen.load(Ordering::SeqCst);
                    while now > m {
                        match max_seen.compare_exchange(m, now, Ordering::SeqCst, Ordering::SeqCst)
                        {
                            Ok(_) => break,
                            Err(actual) => m = actual,
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    active.fetch_sub(1, Ordering::SeqCst);
                    Ok(v)
                }
            },
        ));
        let _ = dispatch_fan_out(producer, target, 3, Value::Null, ctx()).await.unwrap();
        assert!(max_seen.load(Ordering::SeqCst) <= 3);
    }
}
