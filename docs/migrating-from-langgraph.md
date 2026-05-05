# Migrating from LangGraph / LangChain

atomr-agents ports the LangGraph + LangChain feature set into atomr's
actor / strategy idiom. This page maps the concepts you already know
to the equivalents in this framework.

## Concept-mapping table

| LangGraph / LangChain | atomr-agents | Notes |
|---|---|---|
| `Runnable` | `Callable` | trait at `atomr_agents_callable::Callable` |
| `RunnableSequence` (`prompt \| model \| parser`) | `Pipeline::from(prompt).then(model).then(parser).build()` | builder over `CallableHandle` |
| `RunnableParallel({a, b})` | `Pipeline::fan_out_with(vec![("a", a), ("b", b)])` or `fan_out(vec![…])` | concurrent `tokio::spawn` on cloned input |
| `RunnableLambda(fn)` | `Lambda` (alias for `FnCallable`) | `Arc::new(FnCallable::labeled("name", fn))` |
| `RunnablePassthrough` / `.assign(k=fn)` | `Pipeline::passthrough()` / `.assign("k", fn)` | adds key while preserving input dict |
| `RunnableBranch` | `Branch::new(predicate, if_true, if_false)` | inline conditional |
| `.with_retry(stop_after_attempt=N)` | `with_retry(c, RetryPolicy { max_attempts: N, .. })` | exponential backoff with jitter caps |
| `.with_fallbacks([alt1, alt2])` | `with_fallbacks(c, vec![alt1, alt2])` | tries primary, then alternates in order |
| `.with_config(tags=[…], run_name="foo")` | `with_config(c, RunConfig { run_name, tags, metadata })` | propagates into `CallCtx::trace` |
| `astream_events` v2 | `EventBus::subscribe` + `RunTreeBuilder` | flat ordered stream + parent-child tree |
| `StateGraph(MyState)` | `StatefulRunner` over `Dag<Arc<dyn StatefulStep>>` + `StateSchema` | typed channels, not a typed dict |
| `Annotated[list, add_messages]` | `StateSchema::builder().add("messages", AppendMessages)` | reducer attached to channel |
| `add_messages` | `AppendMessages` reducer | append-with-id-dedup |
| `operator.add` (lists) | `AppendList` reducer | concat |
| dict-merge reducer | `MergeMap` reducer | shallow object merge |
| `MemorySaver` / `SqliteSaver` / `PostgresSaver` | `InMemoryCheckpointer` / `SqliteCheckpointer` / `PostgresCheckpointer` | gated on features |
| `thread_id` | `RunId` | newtype on `CallCtx`; checkpoints keyed by `(WorkflowId, RunId, super_step)` |
| `get_state(thread_id)` / `update_state` | `Checkpointer::latest(wf, run)` / `Checkpointer::fork(...)` | fork is the substrate for time-travel |
| `interrupt(payload)` | `ctrl.interrupt(step_id, Some(payload))` from inside an `InterruptibleStep` | persists pause checkpoint, returns `RunOutcome::Paused` |
| `interrupt_before=["x"]` / `interrupt_after=["x"]` | `Interruptible { interrupt_before, interrupt_after }` | static breakpoints |
| `Command(resume=v)` | `Command::Resume(v)` to `Interruptible::resume` | injects value into the paused step |
| `Command(update={k: v})` | `Command::Update(vec![(k, v)])` | edits state and resumes |
| `Command(goto="x")` | `Command::Goto(StepId::new("x"))` | jumps to a step |
| `Send("step", input)` | `dispatch_fan_out(producer, target, n)` | runtime fan-out with bounded concurrency |
| `ToolNode` | parallel tool dispatch in `Agent::run_turn` (built-in) | `tokio::JoinSet` with order-preserved aggregation |
| `tools_condition` | inner loop check on `FinishReason::ToolCalls` | built-in to the agent's per-turn pipeline |
| `Subgraph` | `Subgraph` callable with `input_channels`/`output_channels` | parent declares projection |
| `Store` (long-term) | `LongStore` trait + `InMemoryLongStore` | namespace tuple keys, embedding-indexed search |
| `index=` on `Store.put` | `LongStore::put(.., embedding=Some(v))` | embedding stored alongside value |
| `bind_tools` on a model | `StaticToolStrategy::new(tools)` on the agent | wired in agent construction |
| `with_structured_output(schema)` | `Parser<T>` + `SchemaParser<T: DeserializeOwned>` | wrap in `OutputFixingParser` for auto-repair |
| `PydanticOutputParser` | `SchemaParser<T>` | derives parse via serde |
| `OutputFixingParser` | `OutputFixingParser<P, T>` | re-prompts a `RepairModel` with format hints |
| `RetryWithErrorOutputParser` | `RetryWithErrorParser<P, T>` | re-prompts with the original prompt + failure |
| streaming partial-JSON parse | `StreamingPartialJsonParser` | feed chunks, get incremental `Value`s |
| `ChatPromptTemplate.from_messages([...])` | `ChatPromptTemplate::builder().system().user().placeholder("history")` | builder-style |
| `MessagesPlaceholder("history")` | `MessageTemplate::Placeholder { key: "history" }` (or via `.placeholder("history")`) | inserts message array |
| `FewShotPromptTemplate` | `FewShotChatTemplate { formatter, selector, example_template }` | composes example renderer + main template |
| `LengthBasedExampleSelector` | `LengthBasedSelector` | greedy under `max_tokens` |
| `SemanticSimilarityExampleSelector` | `SemanticSimilaritySelector { embedder, query_key, top_k }` | cosine over `Example.query_text` |
| `BaseRetriever` | `Retriever` | async trait |
| `MultiQueryRetriever` | `MultiQueryRetriever { base, expander }` | bring your own `QueryExpander` |
| `ContextualCompressionRetriever` | `ContextualCompressionRetriever { base, step }` | ships `SentenceFilterCompressor` |
| `ParentDocumentRetriever` | `ParentDocumentRetriever` | register parent + child ids |
| `EnsembleRetriever` | `EnsembleRetriever::with_rrf(members, top_k)` | RRF default `k = 60` |
| `SelfQueryRetriever` | `SelfQueryRetriever { base, parser }` | ships `KeyValueParser` |
| `EmbeddingsFilter` | `EmbeddingsFilter { base, embedder, threshold }` | cosine cutoff |
| `TimeWeightedVectorStoreRetriever` | `TimeWeightedRetriever { base, decay_rate }` | reads `ts_ms` from metadata |
| `RecursiveCharacterTextSplitter` | `RecursiveCharacterSplitter` | greedy with separator fallback |
| `MarkdownHeaderTextSplitter` | `MarkdownHeaderSplitter` | sections by `# / ## / ###` |
| code splitter | `CodeSplitter { lang: CodeLang::{Rust, Python, Js} }` | top-level fn / class boundaries |
| token splitter | `TokenSplitter { max_tokens, overlap_tokens }` | whitespace approximation |
| `SemanticChunker` | `SemanticSplitter::split_async(...)` | embed sentences, break at low-similarity boundaries |
| `CacheBackedEmbeddings` | `CachedEmbedder { inner, cache }` | content-hash key |
| LLM cache (memory) | `InMemoryLlmCache` | hash-keyed `(model, messages, sampling)` |
| Semantic LLM cache | `SemanticLlmCache { embedder, threshold }` | cosine match on prompt |
| Redis cache / SQLite cache | `RedisLlmCache` / `SqliteLlmCache` | feature-gated stubs |
| `create_agent` / `create_react_agent` | `Agent<I, T, Ms, Sk>` | strategy-generic agent struct |
| `@wrap_model_call` / `@wrap_tool_call` middleware | `AgentMiddleware::before_model_call` / `after_model_call` / `before_tool_call` / `after_tool_call` | hook-style; `MiddlewareStack` orders them |
| `@dynamic_prompt` | `AgentMiddleware::dynamic_prompt` | last `Some(_)` wins |
| `@before_agent` / `@after_agent` | `AgentMiddleware::before_agent` / `after_agent` | same names |
| Supervisor pattern | `Team { routing: CapabilityMatchRouter }` + child agents | `route` field on input drives dispatch |
| Swarm pattern | `swarm_loop(agents, &active, ...)` + `HandoffTool` | shared `ActiveAgent` slot |
| Network pattern | same as swarm, no topology constraint | any agent → any agent |
| Hierarchical pattern | nested `Org` → `Department` → `Team` | each level is `Callable` |
| Handoff tool | `HandoffTool::new("target")` (`RichTool`) | returns `ToolReturn::Command(Handoff)` |
| LangSmith tracing | `LangSmithTracer::new(builder, "project", sink)` | shipped with `MemorySink` for tests |
| LangSmith offline eval | `EvalSuite::run(callable)` + `RegressionGate::check` | judge / pairwise / rubric scorers |
| LangSmith pairwise eval | `PairwiseScorer { model, criteria_label }.compare(...)` | A/B preference |
| LLM-as-judge | `LlmJudgeScorer` | first line `pass`/`fail` + justification |
| Annotation queue | `AnnotationQueue` + `InMemoryAnnotationQueue` | trait for pluggable backends |
| LangGraph Studio inspector | `atomr-agents serve` (`Inspector` API in `agents-cli`) | read+resume HTTP endpoints |

## Concrete code translations

### Tool-calling agent

LangGraph:

```python
from langgraph.prebuilt import create_react_agent
agent = create_react_agent(model, tools, prompt=system_prompt)
out = await agent.ainvoke({"messages": [HumanMessage("…")]})
```

atomr-agents:

```rust
let agent = Agent {
    id: AgentId::from("a-1"),
    model: "gpt-4o-mini".into(),
    instructions: ComposedInstructionStrategy::new(
        StaticPersonaStrategy::new(""),
        StaticTaskStrategy(system_prompt.into()),
        StaticBehaviorStrategy("".into()),
    ),
    tools: StaticToolStrategy::new(tools),
    memory: RecencyMemoryStrategy::new(store, 8, 40),
    skills: StaticSkillStrategy::new(vec![]),
    inference,
    bus: EventBus::new(),
    max_tool_iterations: 5,
};
let out = agent.run_turn(user_msg, AgentBudgets::default()).await?;
```

### State + checkpoint + interrupt

LangGraph:

```python
from langgraph.graph import StateGraph
from langgraph.checkpoint.memory import MemorySaver

def my_node(state): return {"messages": [AIMessage("hi")]}

graph = StateGraph(MessagesState)
graph.add_node("step", my_node)
graph.add_edge(START, "step")
checkpointer = MemorySaver()
runnable = graph.compile(checkpointer=checkpointer, interrupt_before=["step"])

await runnable.ainvoke(input, config={"configurable": {"thread_id": "t-1"}})
# … pauses; resume:
await runnable.ainvoke(Command(resume="ok"), config={"configurable": {"thread_id": "t-1"}})
```

atomr-agents:

```rust
let dag: Dag<Arc<dyn InterruptibleStep>> = Dag::builder("step")
    .step("step", Arc::new(FnInterruptStep(|_s, _c| async {
        Ok(vec![("messages".into(), serde_json::json!([{"id": "m1"}]))])
    })))
    .build();

let mut before = HashSet::new();
before.insert(StepId::new("step"));

let runner = Interruptible {
    workflow_id: "wf".into(),
    run_id: "t-1".into(),    // == thread_id
    dag,
    schema,
    checkpointer: Arc::new(InMemoryCheckpointer::new()),
    interrupt_before: before,
    interrupt_after: HashSet::new(),
};

match runner.run().await? {
    RunOutcome::Paused { .. } => {
        let _ = runner.resume(Command::Continue).await?;
    }
    RunOutcome::Done(_) => {}
}
```

### RAG with hybrid retrieval

LangChain:

```python
ensemble = EnsembleRetriever(
    retrievers=[bm25_retriever, vector_retriever],
    weights=[0.5, 0.5],
)
docs = await ensemble.ainvoke(query)
```

atomr-agents:

```rust
let ensemble: Arc<dyn Retriever> = Arc::new(EnsembleRetriever::with_rrf(
    vec![bm25.clone(), dense.clone()],
    /* top_k */ 10,
));
let docs = ensemble.retrieve(query, &ctx).await?;
```

### Auto-repairing structured output

LangChain:

```python
parser = PydanticOutputParser(pydantic_object=Plan)
fixing = OutputFixingParser.from_llm(parser=parser, llm=model)
plan: Plan = fixing.parse(raw_output)
```

atomr-agents:

```rust
let parser = SchemaParser::<Plan>::new("Reply as Plan JSON.");
let fixing: OutputFixingParser<SchemaParser<Plan>, Plan> =
    OutputFixingParser::new(parser, repair_model, /* max_attempts */ 3);
let plan = fixing.parse(&raw_output).await?;
```

## Where to go from here

- [README](../README.md) — quick start.
- [Architecture](architecture.md) — full crate map.
- [`ai-skills/atomr-agents-langgraph-migration/`](../ai-skills/skills/atomr-agents-langgraph-migration/SKILL.md)
  — the same concept map, targeted at AI coding assistants.
