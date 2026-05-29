//! FR-12 — Host trigger runtime: event/webhook triggers, calendar-aware
//! cron triggers, and a runtime control surface (pause/resume/cooldown +
//! per-trigger rate limiting).
//!
//! ## Hedge-fund motivation
//!
//! Real-money trading needs triggers that go well beyond the fixed-interval
//! [`crate::scheduler::Scheduler`] (`every:Ns/m/h/d`):
//!
//! - **Event-driven risk breakers.** On-fill / on-mark / on-breach loops and
//!   broker drop-copy capture are *push* events. [`EventTrigger`] +
//!   [`EventTriggerRegistry`] model the dispatch surface so a webhook / stream
//!   / pub-sub handler can match an inbound event to a registered trigger and
//!   fire a run with the payload delivered to it.
//! - **Trading-calendar cron.** Reconciliation, settlement and venue-session
//!   jobs are calendar-bound ("EOD on business days", "16:00 ET on trading
//!   days"). [`CronTrigger`] supports real cron fields (via the [`cron`] crate)
//!   plus an optional [`TradingCalendar`] so non-trading days (weekends +
//!   holidays) are skipped.
//! - **Instant ops control.** A misconfigured live trigger on real markets must
//!   be pausable / throttleable *instantly* by ops without a redeploy.
//!   [`TriggerControl`] exposes `list_triggers` / `pause` / `resume` /
//!   `set_cooldown`, and a per-trigger [`RateLimit`] caps the fire rate with a
//!   selectable [`OnExceed`] policy.
//!
//! ## Observability
//!
//! Trigger fires and control actions are reported through the local
//! [`TriggerObserver`] trait. A no-op default ([`NoopObserver`]) and an
//! in-memory recorder ([`RecordingObserver`]) are provided.
//!
//! NOTE: a later integration will bridge [`TriggerObserver`] to the
//! observability `Telemetry` / `RunEvent` backbone (FR-19) so trigger
//! lifecycle and control actions land on the shared telemetry topic. This
//! module deliberately keeps the observer local so `crates/host` stays
//! self-contained (per the FR-12 scope constraint).

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Datelike, FixedOffset, NaiveDate, TimeZone, Utc, Weekday};
use cron::Schedule;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::error::{HostError, HostResult};

// ---------------------------------------------------------------------------
// Observability surface (local; bridges to FR-19 Telemetry later).
// ---------------------------------------------------------------------------

/// Observer for trigger fires and control actions.
///
/// This is a small local surface so `crates/host` does not depend on the
/// observability crate. A later integration will bridge this to the
/// observability `Telemetry` / `RunEvent` backbone so trigger lifecycle and
/// control actions become first-class telemetry events.
pub trait TriggerObserver: Send + Sync {
    /// Called when a trigger fires or a control action is applied.
    ///
    /// `action` is one of: `"fire"`, `"pause"`, `"resume"`, `"cooldown"`,
    /// `"rate_limited"` (with `id` the trigger id).
    fn on_trigger_event(&self, id: &str, action: &str);
}

/// A [`TriggerObserver`] that discards every event.
#[derive(Debug, Clone, Default)]
pub struct NoopObserver;

impl TriggerObserver for NoopObserver {
    fn on_trigger_event(&self, _id: &str, _action: &str) {}
}

/// A [`TriggerObserver`] that records `(id, action)` pairs in memory, for tests
/// and ops introspection.
#[derive(Debug, Clone, Default)]
pub struct RecordingObserver {
    events: Arc<Mutex<Vec<(String, String)>>>,
}

impl RecordingObserver {
    /// Construct an empty recorder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshot all recorded `(id, action)` events in order.
    pub fn events(&self) -> Vec<(String, String)> {
        self.events.lock().clone()
    }

    /// Count recorded events for `(id, action)`.
    pub fn count(&self, id: &str, action: &str) -> usize {
        self.events
            .lock()
            .iter()
            .filter(|(i, a)| i == id && a == action)
            .count()
    }
}

impl TriggerObserver for RecordingObserver {
    fn on_trigger_event(&self, id: &str, action: &str) {
        self.events.lock().push((id.to_string(), action.to_string()));
    }
}

// ---------------------------------------------------------------------------
// 1. Event / webhook / stream / pub-sub triggers.
// ---------------------------------------------------------------------------

/// The inbound source an [`EventTrigger`] listens on.
///
/// Real transport wiring (HTTP server, atomr-streams Source, pub/sub client)
/// lives outside this module; the registry only models the *dispatch surface*
/// so a webhook handler can call [`EventTriggerRegistry::fire`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TriggerSource {
    /// An HTTP webhook, matched by request path (e.g. broker drop-copy).
    Webhook {
        /// The webhook path, e.g. `/hooks/broker/fills`.
        path: String,
    },
    /// An atomr-streams topic.
    Stream {
        /// The stream topic name.
        topic: String,
    },
    /// A distributed pub/sub topic.
    PubSub {
        /// The pub/sub topic name.
        topic: String,
    },
}

impl TriggerSource {
    /// Whether this source matches an inbound event source descriptor.
    ///
    /// Equality is exact on the discriminant *and* the path/topic, so a webhook
    /// on `/a` never matches a stream on `/a`.
    pub fn matches(&self, other: &TriggerSource) -> bool {
        self == other
    }
}

/// An event-driven trigger: fires a `call` when a matching inbound event
/// arrives, optionally filtered by a JSON predicate over the payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventTrigger {
    /// Stable trigger id.
    pub id: String,
    /// The inbound source this trigger listens on.
    pub source: TriggerSource,
    /// Optional JSON filter: a subset object that must match the payload
    /// (shallow key/value containment). `None` matches every payload.
    #[serde(default)]
    pub filter: Option<serde_json::Value>,
    /// The call to dispatch (e.g. `{"kind":"agent","id":"risk-breaker"}`).
    pub call: serde_json::Value,
    /// Template merged with the event payload to form the run input.
    #[serde(default)]
    pub input_template: serde_json::Value,
}

/// A record produced when an [`EventTrigger`] fires.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerFired {
    /// The trigger that fired.
    pub trigger_id: String,
    /// The call to dispatch.
    pub call: serde_json::Value,
    /// The resolved run input (`input_template` merged with the payload).
    pub input: serde_json::Value,
    /// The verbatim inbound payload that caused the fire.
    pub payload: serde_json::Value,
}

/// Registry of [`EventTrigger`]s. Match inbound events to triggers and produce
/// [`TriggerFired`] records.
#[derive(Clone)]
pub struct EventTriggerRegistry {
    inner: Arc<Mutex<HashMap<String, EventTrigger>>>,
    observer: Arc<dyn TriggerObserver>,
}

impl Default for EventTriggerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl EventTriggerRegistry {
    /// Construct an empty registry with a no-op observer.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            observer: Arc::new(NoopObserver),
        }
    }

    /// Construct an empty registry with a custom observer.
    pub fn with_observer(observer: Arc<dyn TriggerObserver>) -> Self {
        Self { inner: Arc::new(Mutex::new(HashMap::new())), observer }
    }

    /// Register (or replace) a trigger by id.
    pub fn register(&self, trigger: EventTrigger) {
        self.inner.lock().insert(trigger.id.clone(), trigger);
    }

    /// Remove a trigger by id; returns whether it existed.
    pub fn remove(&self, id: &str) -> bool {
        self.inner.lock().remove(id).is_some()
    }

    /// List registered triggers.
    pub fn list(&self) -> Vec<EventTrigger> {
        let mut out: Vec<_> = self.inner.lock().values().cloned().collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    /// Match an inbound event against registered triggers and fire each one
    /// whose [`TriggerSource`] matches `source_match` and whose `filter`
    /// accepts `payload`. Returns one [`TriggerFired`] per matched trigger.
    pub fn fire(&self, source_match: &TriggerSource, payload: &serde_json::Value) -> Vec<TriggerFired> {
        let triggers: Vec<EventTrigger> = {
            let guard = self.inner.lock();
            guard
                .values()
                .filter(|t| t.source.matches(source_match) && filter_matches(t.filter.as_ref(), payload))
                .cloned()
                .collect()
        };
        let mut out = Vec::with_capacity(triggers.len());
        for t in triggers {
            self.observer.on_trigger_event(&t.id, "fire");
            out.push(TriggerFired {
                trigger_id: t.id.clone(),
                call: t.call.clone(),
                input: merge_input(&t.input_template, payload),
                payload: payload.clone(),
            });
        }
        out.sort_by(|a, b| a.trigger_id.cmp(&b.trigger_id));
        out
    }
}

/// Shallow JSON filter: every key in `filter` must be present in `payload`
/// with an equal value. A `None` filter matches everything. A non-object
/// filter must equal the payload exactly.
fn filter_matches(filter: Option<&serde_json::Value>, payload: &serde_json::Value) -> bool {
    match filter {
        None => true,
        Some(serde_json::Value::Object(want)) => {
            let Some(have) = payload.as_object() else {
                return false;
            };
            want.iter().all(|(k, v)| have.get(k) == Some(v))
        }
        Some(other) => other == payload,
    }
}

/// Merge the event payload into the input template. If both are JSON objects,
/// payload keys are layered on top of the template; otherwise the payload
/// replaces the template when the template is null, else the template wins.
fn merge_input(template: &serde_json::Value, payload: &serde_json::Value) -> serde_json::Value {
    match (template, payload) {
        (serde_json::Value::Object(t), serde_json::Value::Object(p)) => {
            let mut out = t.clone();
            for (k, v) in p {
                out.insert(k.clone(), v.clone());
            }
            serde_json::Value::Object(out)
        }
        (serde_json::Value::Null, p) => p.clone(),
        (t, _) => t.clone(),
    }
}

// ---------------------------------------------------------------------------
// 2. Calendar-aware cron triggers.
// ---------------------------------------------------------------------------

/// A trading calendar: which weekdays are business days, plus an explicit
/// holiday set. Used to make a [`CronTrigger`] fire only on trading days.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradingCalendar {
    /// Explicit non-trading dates (e.g. market holidays).
    pub holidays: BTreeSet<NaiveDate>,
    /// Which weekdays are business days, indexed `[Mon, Tue, Wed, Thu, Fri,
    /// Sat, Sun]`. Default is Mon–Fri.
    pub business_days: [bool; 7],
}

impl Default for TradingCalendar {
    fn default() -> Self {
        Self {
            holidays: BTreeSet::new(),
            // Mon–Fri trading, weekend closed.
            business_days: [true, true, true, true, true, false, false],
        }
    }
}

impl TradingCalendar {
    /// Construct a Mon–Fri calendar with the given holiday set.
    pub fn with_holidays(holidays: impl IntoIterator<Item = NaiveDate>) -> Self {
        Self { holidays: holidays.into_iter().collect(), ..Self::default() }
    }

    /// Whether `date` is a trading day: a configured business weekday that is
    /// not a holiday.
    pub fn is_trading_day(&self, date: NaiveDate) -> bool {
        let idx = match date.weekday() {
            Weekday::Mon => 0,
            Weekday::Tue => 1,
            Weekday::Wed => 2,
            Weekday::Thu => 3,
            Weekday::Fri => 4,
            Weekday::Sat => 5,
            Weekday::Sun => 6,
        };
        self.business_days[idx] && !self.holidays.contains(&date)
    }
}

/// A parsed standard cron expression.
///
/// Wraps the [`cron`] crate's [`Schedule`]. The `cron` crate expects a
/// 6- or 7-field expression (`sec min hour day-of-month month day-of-week
/// [year]`).
#[derive(Debug, Clone)]
pub struct CronExpr {
    raw: String,
    schedule: Schedule,
}

impl CronExpr {
    /// Parse a standard cron expression. Returns a [`HostError::Scheduler`] on
    /// invalid syntax.
    pub fn parse(expr: &str) -> HostResult<Self> {
        let schedule = Schedule::from_str(expr)
            .map_err(|e| HostError::Scheduler(format!("invalid cron `{expr}`: {e}")))?;
        Ok(Self { raw: expr.to_string(), schedule })
    }

    /// The raw expression string.
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// Next fire time strictly after `from` in the given fixed-offset timezone,
    /// as a unix-ms timestamp.
    fn next_after_ms(&self, from_ms: i64, offset: FixedOffset) -> Option<i64> {
        let from_utc: DateTime<Utc> = Utc.timestamp_millis_opt(from_ms).single()?;
        let from_local = from_utc.with_timezone(&offset);
        let next = self.schedule.after(&from_local).next()?;
        Some(next.with_timezone(&Utc).timestamp_millis())
    }
}

impl Serialize for CronExpr {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.raw)
    }
}

impl<'de> Deserialize<'de> for CronExpr {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        CronExpr::parse(&raw).map_err(serde::de::Error::custom)
    }
}

/// A calendar-aware cron trigger.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronTrigger {
    /// Stable trigger id.
    pub id: String,
    /// The cron schedule.
    pub schedule: CronExpr,
    /// Optional trading calendar. When set, fires that land on a non-trading
    /// day (weekend / holiday) are skipped and the next trading-day occurrence
    /// is used instead.
    #[serde(default)]
    pub calendar: Option<TradingCalendar>,
    /// Timezone offset, in minutes east of UTC, the cron fields are evaluated
    /// in (e.g. `-300` for US Eastern Standard Time).
    #[serde(default)]
    pub tz_offset_minutes: i32,
    /// Whether the trigger is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

impl CronTrigger {
    /// Compute the next fire time strictly after `from_unix_ms`, as unix-ms.
    ///
    /// Returns `None` for a disabled trigger or when the schedule has no
    /// further occurrences. When a [`TradingCalendar`] is set, occurrences that
    /// fall on a non-trading day are skipped and search advances to the next
    /// occurrence on a trading day.
    pub fn next_fire_after(&self, from_unix_ms: i64) -> Option<i64> {
        if !self.enabled {
            return None;
        }
        let offset = FixedOffset::east_opt(self.tz_offset_minutes * 60)?;
        let mut cursor = from_unix_ms;
        // Bound the search so a calendar that never trades cannot loop forever
        // (e.g. far more days than a few years of occurrences).
        for _ in 0..4096 {
            let next = self.schedule.next_after_ms(cursor, offset)?;
            match &self.calendar {
                None => return Some(next),
                Some(cal) => {
                    let local: DateTime<FixedOffset> =
                        Utc.timestamp_millis_opt(next).single()?.with_timezone(&offset);
                    if cal.is_trading_day(local.date_naive()) {
                        return Some(next);
                    }
                    // Skip the rest of this non-trading day: advance the cursor
                    // to the end of the local day so we don't re-test the same
                    // day's later occurrences one by one.
                    let day_end = local
                        .date_naive()
                        .and_hms_opt(23, 59, 59)
                        .and_then(|d| offset.from_local_datetime(&d).single())?;
                    cursor = day_end.with_timezone(&Utc).timestamp_millis();
                    // Guard against the schedule returning a time at/just before
                    // our new cursor (cron `.after` is strict, but be safe).
                    if cursor <= next {
                        cursor = next + 1;
                    }
                }
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------
// 4. Rate limiting (sliding window).
// ---------------------------------------------------------------------------

/// Policy applied when a [`RateLimit`] window is exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnExceed {
    /// Drop the fire.
    Drop,
    /// Enqueue the fire to be released when the window frees up.
    Queue,
    /// Signal backpressure to the caller.
    Backpressure,
}

/// A sliding-window rate limit: at most `max_fires` within any `per` window.
#[derive(Debug, Clone)]
pub struct RateLimit {
    /// Maximum fires allowed within the window.
    pub max_fires: u32,
    /// The window duration.
    pub per: Duration,
    /// What to do when the window is full.
    pub on_exceed: OnExceed,
    /// Timestamps (unix-ms) of fires within the current window.
    window: VecDeque<i64>,
    /// Queued fire timestamps (only used by [`OnExceed::Queue`]).
    queued: VecDeque<i64>,
}

/// The decision returned by [`RateLimit::check_and_record`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateDecision {
    /// The fire is allowed and was recorded.
    Allow,
    /// The fire was dropped (window full, [`OnExceed::Drop`]).
    Dropped,
    /// The fire was queued (window full, [`OnExceed::Queue`]).
    Queued,
    /// The fire is backpressured (window full, [`OnExceed::Backpressure`]).
    Backpressured,
}

impl RateLimit {
    /// Construct a rate limit.
    pub fn new(max_fires: u32, per: Duration, on_exceed: OnExceed) -> Self {
        Self { max_fires, per, on_exceed, window: VecDeque::new(), queued: VecDeque::new() }
    }

    fn evict_before(&mut self, now_ms: i64) {
        let horizon = now_ms - self.per.as_millis() as i64;
        while let Some(&front) = self.window.front() {
            if front <= horizon {
                self.window.pop_front();
            } else {
                break;
            }
        }
    }

    /// Check whether a fire at `now_ms` is allowed under the sliding window and
    /// record it if so. Applies the configured [`OnExceed`] policy when the
    /// window is full.
    pub fn check_and_record(&mut self, now_ms: i64) -> RateDecision {
        self.evict_before(now_ms);
        if self.window.len() < self.max_fires as usize {
            self.window.push_back(now_ms);
            return RateDecision::Allow;
        }
        match self.on_exceed {
            OnExceed::Drop => RateDecision::Dropped,
            OnExceed::Queue => {
                self.queued.push_back(now_ms);
                RateDecision::Queued
            }
            OnExceed::Backpressure => RateDecision::Backpressured,
        }
    }

    /// Number of fires currently queued (for [`OnExceed::Queue`]).
    pub fn queued_len(&self) -> usize {
        self.queued.len()
    }
}

// ---------------------------------------------------------------------------
// 3. Runtime control surface.
// ---------------------------------------------------------------------------

/// What kind of trigger a [`TriggerStatus`] describes.
const KIND_EVENT: &str = "event";
const KIND_CRON: &str = "cron";

/// A snapshot of a trigger's runtime state, returned by
/// [`TriggerControl::list_triggers`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerStatus {
    /// Trigger id.
    pub id: String,
    /// `"event"` or `"cron"`.
    pub kind: String,
    /// Whether firing is currently enabled (not paused).
    pub enabled: bool,
    /// Cooldown between fires, in milliseconds.
    pub cooldown_ms: u64,
    /// Total number of fires so far.
    pub fires: u64,
    /// Unix-ms of the most recent fire, if any.
    pub last_fire_ms: Option<i64>,
}

/// Per-trigger mutable runtime state held by [`TriggerControl`].
struct TriggerEntry {
    kind: &'static str,
    enabled: bool,
    cooldown: Duration,
    fires: u64,
    last_fire_ms: Option<i64>,
    rate_limit: Option<RateLimit>,
}

impl TriggerEntry {
    fn new(kind: &'static str) -> Self {
        Self {
            kind,
            enabled: true,
            cooldown: Duration::ZERO,
            fires: 0,
            last_fire_ms: None,
            rate_limit: None,
        }
    }

    fn status(&self, id: &str) -> TriggerStatus {
        TriggerStatus {
            id: id.to_string(),
            kind: self.kind.to_string(),
            enabled: self.enabled,
            cooldown_ms: self.cooldown.as_millis() as u64,
            fires: self.fires,
            last_fire_ms: self.last_fire_ms,
        }
    }
}

/// The outcome of an attempted fire, accounting for enabled/cooldown/rate-limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FireOutcome {
    /// The fire is permitted and was recorded.
    Fired,
    /// The trigger is paused.
    Paused,
    /// The fire is within the cooldown window since the last fire.
    CoolingDown,
    /// The fire was rate-limited; carries the rate decision.
    RateLimited(RateDecision),
}

/// Runtime control surface over all registered triggers.
///
/// Holds per-trigger enabled/cooldown/rate-limit/fire-stats so ops can
/// introspect and pause/resume/throttle a specific trigger at runtime without
/// a redeploy. Control actions and fires are reported to the [`TriggerObserver`].
#[derive(Clone)]
pub struct TriggerControl {
    inner: Arc<Mutex<HashMap<String, TriggerEntry>>>,
    observer: Arc<dyn TriggerObserver>,
}

impl Default for TriggerControl {
    fn default() -> Self {
        Self::new()
    }
}

impl TriggerControl {
    /// Construct an empty control surface with a no-op observer.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            observer: Arc::new(NoopObserver),
        }
    }

    /// Construct with a custom observer.
    pub fn with_observer(observer: Arc<dyn TriggerObserver>) -> Self {
        Self { inner: Arc::new(Mutex::new(HashMap::new())), observer }
    }

    /// Register an event trigger for control.
    pub fn register_event(&self, id: impl Into<String>) {
        self.inner.lock().insert(id.into(), TriggerEntry::new(KIND_EVENT));
    }

    /// Register a cron trigger for control. The trigger's `enabled` flag seeds
    /// the control entry.
    pub fn register_cron(&self, trigger: &CronTrigger) {
        let mut entry = TriggerEntry::new(KIND_CRON);
        entry.enabled = trigger.enabled;
        self.inner.lock().insert(trigger.id.clone(), entry);
    }

    /// Remove a trigger from control; returns whether it existed.
    pub fn remove(&self, id: &str) -> bool {
        self.inner.lock().remove(id).is_some()
    }

    /// Attach (or replace) a per-trigger [`RateLimit`].
    pub fn set_rate_limit(&self, id: &str, limit: RateLimit) -> bool {
        let mut guard = self.inner.lock();
        match guard.get_mut(id) {
            Some(e) => {
                e.rate_limit = Some(limit);
                true
            }
            None => false,
        }
    }

    /// List the status of every registered trigger, sorted by id.
    pub fn list_triggers(&self) -> Vec<TriggerStatus> {
        let guard = self.inner.lock();
        let mut out: Vec<_> = guard.iter().map(|(id, e)| e.status(id)).collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    /// Fetch the status of a single trigger.
    pub fn status(&self, id: &str) -> Option<TriggerStatus> {
        self.inner.lock().get(id).map(|e| e.status(id))
    }

    /// Pause a trigger (disables firing) without a redeploy. Returns whether it
    /// existed.
    pub fn pause(&self, id: &str) -> bool {
        let changed = {
            let mut guard = self.inner.lock();
            match guard.get_mut(id) {
                Some(e) => {
                    e.enabled = false;
                    true
                }
                None => false,
            }
        };
        if changed {
            self.observer.on_trigger_event(id, "pause");
        }
        changed
    }

    /// Resume a paused trigger. Returns whether it existed.
    pub fn resume(&self, id: &str) -> bool {
        let changed = {
            let mut guard = self.inner.lock();
            match guard.get_mut(id) {
                Some(e) => {
                    e.enabled = true;
                    true
                }
                None => false,
            }
        };
        if changed {
            self.observer.on_trigger_event(id, "resume");
        }
        changed
    }

    /// Set the cooldown between fires for a trigger. Returns whether it existed.
    pub fn set_cooldown(&self, id: &str, cooldown: Duration) -> bool {
        let changed = {
            let mut guard = self.inner.lock();
            match guard.get_mut(id) {
                Some(e) => {
                    e.cooldown = cooldown;
                    true
                }
                None => false,
            }
        };
        if changed {
            self.observer.on_trigger_event(id, "cooldown");
        }
        changed
    }

    /// Attempt to fire a trigger at `now_ms`, honoring paused state, cooldown,
    /// and any attached [`RateLimit`]. On success the fire is recorded
    /// (incrementing the fire count + last-fire timestamp) and the observer is
    /// notified with `"fire"`; on rate-limit rejection the observer is notified
    /// with `"rate_limited"`.
    pub fn try_fire(&self, id: &str, now_ms: i64) -> FireOutcome {
        let outcome = {
            let mut guard = self.inner.lock();
            let Some(e) = guard.get_mut(id) else {
                return FireOutcome::Paused; // unknown id treated as not firable
            };
            if !e.enabled {
                FireOutcome::Paused
            } else if let Some(last) = e.last_fire_ms {
                if now_ms - last < e.cooldown.as_millis() as i64 {
                    FireOutcome::CoolingDown
                } else {
                    Self::apply_rate_then_record(e, now_ms)
                }
            } else {
                Self::apply_rate_then_record(e, now_ms)
            }
        };
        match outcome {
            FireOutcome::Fired => self.observer.on_trigger_event(id, "fire"),
            FireOutcome::RateLimited(_) => self.observer.on_trigger_event(id, "rate_limited"),
            _ => {}
        }
        outcome
    }

    fn apply_rate_then_record(e: &mut TriggerEntry, now_ms: i64) -> FireOutcome {
        if let Some(rl) = e.rate_limit.as_mut() {
            match rl.check_and_record(now_ms) {
                RateDecision::Allow => {}
                other => return FireOutcome::RateLimited(other),
            }
        }
        e.fires += 1;
        e.last_fire_ms = Some(now_ms);
        FireOutcome::Fired
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Timelike;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    // --- Event triggers ----------------------------------------------------

    #[test]
    fn webhook_event_fires_matching_trigger_with_payload() {
        let reg = EventTriggerRegistry::new();
        reg.register(EventTrigger {
            id: "on-fill".into(),
            source: TriggerSource::Webhook { path: "/hooks/fills".into() },
            filter: None,
            call: serde_json::json!({"kind":"agent","id":"risk-breaker"}),
            input_template: serde_json::json!({"reason":"fill"}),
        });
        // Non-matching source must not fire.
        let none = reg.fire(
            &TriggerSource::Webhook { path: "/hooks/other".into() },
            &serde_json::json!({"qty": 100}),
        );
        assert!(none.is_empty());

        let payload = serde_json::json!({"qty": 100, "symbol": "AAPL"});
        let fired = reg.fire(&TriggerSource::Webhook { path: "/hooks/fills".into() }, &payload);
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].trigger_id, "on-fill");
        assert_eq!(fired[0].payload, payload);
        // input_template merged with payload.
        assert_eq!(fired[0].input["reason"], serde_json::json!("fill"));
        assert_eq!(fired[0].input["symbol"], serde_json::json!("AAPL"));
        assert_eq!(fired[0].input["qty"], serde_json::json!(100));
    }

    #[test]
    fn stream_event_fires_and_filter_excludes() {
        let reg = EventTriggerRegistry::new();
        reg.register(EventTrigger {
            id: "breach".into(),
            source: TriggerSource::Stream { topic: "marks".into() },
            filter: Some(serde_json::json!({"breach": true})),
            call: serde_json::json!({"kind":"agent","id":"halt"}),
            input_template: serde_json::json!({}),
        });
        // Filter rejects (no breach key / false).
        let no = reg.fire(
            &TriggerSource::Stream { topic: "marks".into() },
            &serde_json::json!({"breach": false, "mark": 10.0}),
        );
        assert!(no.is_empty());
        // Filter accepts.
        let yes = reg.fire(
            &TriggerSource::Stream { topic: "marks".into() },
            &serde_json::json!({"breach": true, "mark": 99.0}),
        );
        assert_eq!(yes.len(), 1);
        assert_eq!(yes[0].trigger_id, "breach");
        assert_eq!(yes[0].payload["mark"], serde_json::json!(99.0));
    }

    #[test]
    fn event_fire_notifies_observer() {
        let obs = Arc::new(RecordingObserver::new());
        let reg = EventTriggerRegistry::with_observer(obs.clone());
        reg.register(EventTrigger {
            id: "t1".into(),
            source: TriggerSource::PubSub { topic: "ticks".into() },
            filter: None,
            call: serde_json::json!({}),
            input_template: serde_json::json!({}),
        });
        reg.fire(&TriggerSource::PubSub { topic: "ticks".into() }, &serde_json::json!({}));
        assert_eq!(obs.count("t1", "fire"), 1);
    }

    // --- Calendar / cron ---------------------------------------------------

    #[test]
    fn calendar_is_trading_day_weekend_and_holiday_aware() {
        // 2024-07-04 (Thu) is a US holiday; 2024-07-06 is a Saturday.
        let cal = TradingCalendar::with_holidays([d(2024, 7, 4)]);
        assert!(cal.is_trading_day(d(2024, 7, 3))); // Wed
        assert!(!cal.is_trading_day(d(2024, 7, 4))); // holiday
        assert!(cal.is_trading_day(d(2024, 7, 5))); // Fri
        assert!(!cal.is_trading_day(d(2024, 7, 6))); // Sat
        assert!(!cal.is_trading_day(d(2024, 7, 7))); // Sun
    }

    #[test]
    fn eod_cron_on_trading_days_skips_holiday() {
        // "EOD" = 16:00:00 every day, evaluated in UTC for test simplicity.
        // cron crate format: sec min hour dom month dow
        let trigger = CronTrigger {
            id: "eod".into(),
            schedule: CronExpr::parse("0 0 16 * * *").unwrap(),
            calendar: Some(TradingCalendar::with_holidays([d(2024, 7, 4)])),
            tz_offset_minutes: 0,
            enabled: true,
        };
        // Start just after Wed 2024-07-03 16:00 UTC. Next plain occurrence is
        // Thu 07-04 16:00 (a holiday) -> must be skipped to Fri 07-05 16:00.
        let from = Utc
            .with_ymd_and_hms(2024, 7, 3, 16, 0, 1)
            .single()
            .unwrap()
            .timestamp_millis();
        let next_ms = trigger.next_fire_after(from).expect("a next fire");
        let next = Utc.timestamp_millis_opt(next_ms).single().unwrap();
        assert_eq!(next.date_naive(), d(2024, 7, 5), "holiday Thu skipped, fires Fri");
        assert_eq!(next.hour(), 16);
        assert_eq!(next.minute(), 0);
    }

    #[test]
    fn eod_cron_skips_weekend() {
        let trigger = CronTrigger {
            id: "eod".into(),
            schedule: CronExpr::parse("0 0 16 * * *").unwrap(),
            calendar: Some(TradingCalendar::default()), // Mon-Fri
            tz_offset_minutes: 0,
            enabled: true,
        };
        // Friday 2024-07-05 16:00:01 -> next must be Monday 07-08 (skip Sat/Sun).
        let from = Utc
            .with_ymd_and_hms(2024, 7, 5, 16, 0, 1)
            .single()
            .unwrap()
            .timestamp_millis();
        let next_ms = trigger.next_fire_after(from).unwrap();
        let next = Utc.timestamp_millis_opt(next_ms).single().unwrap();
        assert_eq!(next.date_naive(), d(2024, 7, 8), "weekend skipped, fires Monday");
    }

    #[test]
    fn cron_without_calendar_uses_raw_schedule() {
        let trigger = CronTrigger {
            id: "raw".into(),
            schedule: CronExpr::parse("0 0 16 * * *").unwrap(),
            calendar: None,
            tz_offset_minutes: 0,
            enabled: true,
        };
        // From Sat 07-06 12:00 the next plain occurrence is Sat 07-06 16:00.
        let from = Utc
            .with_ymd_and_hms(2024, 7, 6, 12, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        let next_ms = trigger.next_fire_after(from).unwrap();
        let next = Utc.timestamp_millis_opt(next_ms).single().unwrap();
        assert_eq!(next.date_naive(), d(2024, 7, 6));
        assert_eq!(next.hour(), 16);
    }

    #[test]
    fn disabled_cron_never_fires() {
        let trigger = CronTrigger {
            id: "off".into(),
            schedule: CronExpr::parse("0 0 16 * * *").unwrap(),
            calendar: None,
            tz_offset_minutes: 0,
            enabled: false,
        };
        assert!(trigger.next_fire_after(0).is_none());
    }

    #[test]
    fn cron_invalid_expression_errors() {
        assert!(CronExpr::parse("not a cron").is_err());
    }

    #[test]
    fn cron_expr_serde_roundtrip() {
        let e = CronExpr::parse("0 0 16 * * *").unwrap();
        let s = serde_json::to_string(&e).unwrap();
        assert_eq!(s, "\"0 0 16 * * *\"");
        let back: CronExpr = serde_json::from_str(&s).unwrap();
        assert_eq!(back.as_str(), "0 0 16 * * *");
    }

    #[test]
    fn cron_respects_tz_offset() {
        // 16:00 in UTC-5 == 21:00 UTC.
        let trigger = CronTrigger {
            id: "et".into(),
            schedule: CronExpr::parse("0 0 16 * * *").unwrap(),
            calendar: None,
            tz_offset_minutes: -5 * 60,
            enabled: true,
        };
        let from = Utc
            .with_ymd_and_hms(2024, 7, 3, 0, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        let next = Utc
            .timestamp_millis_opt(trigger.next_fire_after(from).unwrap())
            .single()
            .unwrap();
        assert_eq!(next.hour(), 21, "16:00 ET is 21:00 UTC");
    }

    // --- Rate limiting -----------------------------------------------------

    #[test]
    fn rate_limit_drop_policy() {
        let mut rl = RateLimit::new(2, Duration::from_secs(10), OnExceed::Drop);
        assert_eq!(rl.check_and_record(0), RateDecision::Allow);
        assert_eq!(rl.check_and_record(1), RateDecision::Allow);
        assert_eq!(rl.check_and_record(2), RateDecision::Dropped);
        // After window slides past the first two, allowed again.
        assert_eq!(rl.check_and_record(10_001), RateDecision::Allow);
    }

    #[test]
    fn rate_limit_queue_policy() {
        let mut rl = RateLimit::new(1, Duration::from_secs(10), OnExceed::Queue);
        assert_eq!(rl.check_and_record(0), RateDecision::Allow);
        assert_eq!(rl.check_and_record(1), RateDecision::Queued);
        assert_eq!(rl.check_and_record(2), RateDecision::Queued);
        assert_eq!(rl.queued_len(), 2);
    }

    #[test]
    fn rate_limit_backpressure_policy() {
        let mut rl = RateLimit::new(1, Duration::from_secs(10), OnExceed::Backpressure);
        assert_eq!(rl.check_and_record(0), RateDecision::Allow);
        assert_eq!(rl.check_and_record(1), RateDecision::Backpressured);
    }

    // --- Control surface ---------------------------------------------------

    #[test]
    fn pause_resume_changes_status_without_redeploy() {
        let obs = Arc::new(RecordingObserver::new());
        let ctrl = TriggerControl::with_observer(obs.clone());
        ctrl.register_event("breaker");
        assert!(ctrl.status("breaker").unwrap().enabled);

        assert_eq!(ctrl.try_fire("breaker", 0), FireOutcome::Fired);
        assert_eq!(ctrl.status("breaker").unwrap().fires, 1);

        assert!(ctrl.pause("breaker"));
        assert!(!ctrl.status("breaker").unwrap().enabled);
        assert_eq!(ctrl.try_fire("breaker", 100), FireOutcome::Paused);
        // fires unchanged while paused.
        assert_eq!(ctrl.status("breaker").unwrap().fires, 1);

        assert!(ctrl.resume("breaker"));
        assert_eq!(ctrl.try_fire("breaker", 200), FireOutcome::Fired);
        assert_eq!(ctrl.status("breaker").unwrap().fires, 2);

        assert_eq!(obs.count("breaker", "pause"), 1);
        assert_eq!(obs.count("breaker", "resume"), 1);
        assert_eq!(obs.count("breaker", "fire"), 2);
    }

    #[test]
    fn cooldown_blocks_fire_within_window() {
        let ctrl = TriggerControl::new();
        ctrl.register_event("t");
        assert!(ctrl.set_cooldown("t", Duration::from_millis(1000)));
        assert_eq!(ctrl.status("t").unwrap().cooldown_ms, 1000);

        assert_eq!(ctrl.try_fire("t", 0), FireOutcome::Fired);
        assert_eq!(ctrl.try_fire("t", 500), FireOutcome::CoolingDown);
        assert_eq!(ctrl.try_fire("t", 1000), FireOutcome::Fired);
        assert_eq!(ctrl.status("t").unwrap().fires, 2);
        assert_eq!(ctrl.status("t").unwrap().last_fire_ms, Some(1000));
    }

    #[test]
    fn control_rate_limit_applies_drop_policy() {
        let obs = Arc::new(RecordingObserver::new());
        let ctrl = TriggerControl::with_observer(obs.clone());
        ctrl.register_event("t");
        ctrl.set_rate_limit("t", RateLimit::new(1, Duration::from_secs(10), OnExceed::Drop));
        assert_eq!(ctrl.try_fire("t", 0), FireOutcome::Fired);
        assert_eq!(
            ctrl.try_fire("t", 1),
            FireOutcome::RateLimited(RateDecision::Dropped)
        );
        // Dropped fire does not increment the counter.
        assert_eq!(ctrl.status("t").unwrap().fires, 1);
        assert_eq!(obs.count("t", "rate_limited"), 1);
    }

    #[test]
    fn list_triggers_reports_kind_and_sorted() {
        let ctrl = TriggerControl::new();
        ctrl.register_event("z-event");
        ctrl.register_cron(&CronTrigger {
            id: "a-cron".into(),
            schedule: CronExpr::parse("0 0 16 * * *").unwrap(),
            calendar: None,
            tz_offset_minutes: 0,
            enabled: true,
        });
        let list = ctrl.list_triggers();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, "a-cron");
        assert_eq!(list[0].kind, "cron");
        assert_eq!(list[1].id, "z-event");
        assert_eq!(list[1].kind, "event");
    }
}
