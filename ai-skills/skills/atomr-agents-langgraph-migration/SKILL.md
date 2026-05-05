---
name: atomr-agents-langgraph-migration
description: Use when porting LangGraph or LangChain code to atomr-agents — translating `StateGraph` / `Runnable` / `RunnableSequence` / `MemorySaver` / `interrupt` / `Command` / `ToolNode` / retriever zoo / output parsers / `create_agent` middleware. Triggers on porting an existing LangChain / LangGraph project, or asking "how do I do X from LangGraph in atomr-agents".
---

# Migrating from LangGraph / LangChain

atomr-agents ports the LangGraph + LangChain feature set into
atomr's actor / strategy idiom. This skill is the concept map for
porting existing code.

## Concept-mapping table (essential)

| LangChain / LangGraph | atomr-agents |
|---|---|
| `Runnable` | `Callable` (`atomr_agents_callable::Callable`) |
| `prompt \| model \| parser` | `Pipeline::from(prompt).then(model).then(parser).build()` |
| `RunnableParallel({a, b})` | `Pipeline::fan_out_with(...)` or `fan_out(...)` |
| `RunnableLambda(fn)` | `Lambda` / `FnCallable` |
| `RunnablePassthrough` / `.assign(k=fn)` | `Pipeline::passthrough()` / `.assign("k", fn)` |
| `RunnableBranch` | `Branch::new(predicate, if_true, if_false)` |
| `.with_retry(stop_after_attempt=N)` | `with_retry(c, RetryPolicy { max_attempts: N, .. })` |
| `.with_fallbacks([alt1, alt2])` | `with_fallbacks(c, vec![alt1, alt2])` |
| `.with_config(tags, run_name)` | `with_config(c, RunConfig { run_name, tags, .. })` |
| `astream_events` v2 | `EventBus::subscribe` + `RunTreeBuilder` |
| `StateGraph(MyState)` | `StatefulRunner` over `Dag<Arc<dyn StatefulStep>>` + `StateSchema` |
| `Annotated[list, add_messages]` | `StateSchema::builder().add("messages", AppendMessages)` |
| `MemorySaver` / `SqliteSaver` / `PostgresSaver` | `InMemoryCheckpointer` / `SqliteCheckpointer` / `PostgresCheckpointer` (feature-gated) |
| `thread_id` | `RunId` (newtype on `CallCtx`); checkpoints keyed by `(WorkflowId, RunId, super_step)` |
| `get_state` / `update_state` / `get_state_history` | `Checkpointer::latest` / `Checkpointer::fork` / `Checkpointer::list` |
| `interrupt(payload)` | `ctrl.interrupt(step_id, Some(payload))` from inside an `InterruptibleStep` |
| `interrupt_before=["x"]` | `Interruptible { interrupt_before: HashSet::from([StepId::new("x")]), .. }` |
| `Command(resume=v)` | `Command::Resume(v)` to `Interruptible::resume` |
| `Command(update={k: v})` | `Command::Update(vec![(k, v)])` |
| `Command(goto="x")` | `Command::Goto(StepId::new("x"))` |
| `Send("step", input)` | `dispatch_fan_out(producer, target, n)` |
| `ToolNode` | parallel tool dispatch in `Agent::run_turn` (built-in) |
| `tools_condition` | inner check on `FinishReason::ToolCalls` (built-in) |
| `Subgraph` | `Subgraph` callable with `input_channels` / `output_channels` |
| `Store` (long-term) | `LongStore` trait + `InMemoryLongStore` |
| `bind_tools` on a model | `StaticToolStrategy::new(tools)` on the agent |
| `with_structured_output(schema)` | `Parser<T>` + `SchemaParser<T: DeserializeOwned>` |
| `PydanticOutputParser` | `SchemaParser<T>` |
| `OutputFixingParser` | `OutputFixingParser<P, T>` |
| `RetryWithErrorOutputParser` | `RetryWithErrorParser<P, T>` |
| streaming partial-JSON | `StreamingPartialJsonParser` |
| `ChatPromptTemplate.from_messages([...])` | `ChatPromptTemplate::builder().system().user().placeholder("history")` |
| `MessagesPlaceholder("history")` | `.placeholder("history")` |
| `FewShotPromptTemplate` | `FewShotChatTemplate { formatter, selector, example_template }` |
| `LengthBasedExampleSelector` | `LengthBasedSelector` |
| `SemanticSimilarityExampleSelector` | `SemanticSimilaritySelector` |
| `BaseRetriever` | `Retriever` |
| `MultiQueryRetriever` | `MultiQueryRetriever` |
| `ContextualCompressionRetriever` | `ContextualCompressionRetriever` |
| `ParentDocumentRetriever` | `ParentDocumentRetriever` |
| `EnsembleRetriever` | `EnsembleRetriever::with_rrf(...)` |
| `SelfQueryRetriever` | `SelfQueryRetriever` |
| `EmbeddingsFilter` | `EmbeddingsFilter` |
| `TimeWeightedVectorStoreRetriever` | `TimeWeightedRetriever` |
| `RecursiveCharacterTextSplitter` | `RecursiveCharacterSplitter` |
| `MarkdownHeaderTextSplitter` | `MarkdownHeaderSplitter` |
| `SemanticChunker` | `SemanticSplitter::split_async(...)` |
| `CacheBackedEmbeddings` | `CachedEmbedder { inner, cache }` |
| In-memory LLM cache | `InMemoryLlmCache` |
| Semantic LLM cache | `SemanticLlmCache { embedder, threshold }` |
| `create_agent` | `Agent<I, T, Ms, Sk>` |
| `@wrap_model_call` / `@wrap_tool_call` | `AgentMiddleware::before/after_model_call` / `before/after_tool_call` |
| `@dynamic_prompt` | `AgentMiddleware::dynamic_prompt` |
| Supervisor pattern | `Team { routing: CapabilityMatchRouter }` + child agents |
| Swarm pattern | `swarm_loop(agents, &active, ...)` + `HandoffTool` |
| Handoff tool | `HandoffTool::new("target")` (`RichTool`) |
| LangSmith tracing | `LangSmithTracer::new(builder, "project", sink)` |
| Pairwise eval | `PairwiseScorer { model, criteria_label }` |
| LLM-as-judge | `LlmJudgeScorer` |
| Annotation queue | `AnnotationQueue` + `InMemoryAnnotationQueue` |

## Concrete translations

### LCEL chain

LangChain:

```python
chain = prompt | model.with_retry(stop_after_attempt=3) | parser
out = await chain.ainvoke(input)
```

atomr-agents:

```rust
use atomr_agents::callable::{Pipeline, with_retry, RetryPolicy};

let chain = Pipeline::from(prompt)
    .then(with_retry(model, RetryPolicy { max_attempts: 3, ..Default::default() }))
    .then(parser)
    .build();
let out = chain.call(input, ctx).await?;
```

### create_react_agent

LangGraph:

```python
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

### StateGraph + interrupt

LangGraph:

```python
graph = StateGraph(MessagesState)
graph.add_node("step", my_node)
graph.add_edge(START, "step")
checkpointer = MemorySaver()
runnable = graph.compile(checkpointer=checkpointer, interrupt_before=["step"])

await runnable.ainvoke(input, config={"configurable": {"thread_id": "t-1"}})
await runnable.ainvoke(Command(resume="ok"), config={"configurable": {"thread_id": "t-1"}})
```

atomr-agents:

```rust
use std::collections::HashSet;
use atomr_agents_workflow::{Command, Dag, Interruptible, RunOutcome, StepId};

let dag: Dag<Arc<dyn InterruptibleStep>> = Dag::builder("step")
    .step("step", my_step)
    .build();

let mut before = HashSet::new();
before.insert(StepId::new("step"));

let runner = Interruptible {
    workflow_id: "wf".into(),
    run_id: "t-1".into(),  // == thread_id
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

### Hybrid retrieval

LangChain:

```python
ensemble = EnsembleRetriever(
    retrievers=[bm25_retriever, vector_retriever],
    weights=[0.5, 0.5],
)
```

atomr-agents:

```rust
let ensemble: Arc<dyn Retriever> = Arc::new(EnsembleRetriever::with_rrf(
    vec![bm25.clone(), dense.clone()],
    /* top_k */ 10,
));
```

(RRF is the canonical hybrid weighting; `k = 60` by default.)

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

## What's different (intentional)

- **Strategies, not classes.** atomr-agents uses generic strategy
  traits rather than runtime-polymorphic class hierarchies — the
  hot path is monomorphized.
- **`thread_id` → `RunId`.** A newtype on `CallCtx`. Same role.
- **Checkpoint key.** atomr-agents keys checkpoints by `(WorkflowId,
  RunId, super_step)` — three fields, not one.
- **Reducers in the schema, not annotations.** `StateSchema` carries
  reducers explicitly per channel; no `Annotated[list, add_messages]`
  sleight-of-hand.
- **No "configurable" runtime swap.** Use builder-style with
  `with_config(c, RunConfig { ... })`; runtime model swaps go
  through `with_fallbacks`.
- **Tool dispatch is `JoinSet`-parallel by default.** No need to
  wrap with `ToolNode` — it's the agent's per-turn behavior.

## What's missing (deliberately deferred)

- **Visual no-code builder.** Studio inspector is read+resume only.
- **Synthetic data generation chains.** Build over R9 (parsers).
- **Vendor-specific self-querying language databases beyond
  `SelfQueryRetriever`'s flat key:value parser.**
- **Annotation review UI.** `AnnotationQueue` is a queue; the UI
  layer is your call.

## Canonical references

- [`docs/migrating-from-langgraph.md`](https://github.com/rustakka/atomr-agents/blob/main/docs/migrating-from-langgraph.md) — full concept map + more code translations
- [`docs/architecture.md`](https://github.com/rustakka/atomr-agents/blob/main/docs/architecture.md) — where each port lives in the crate stack
- [`docs/feature-matrix.md`](https://github.com/rustakka/atomr-agents/blob/main/docs/feature-matrix.md) — feature flags

## Common mistakes during migration

- **Reaching for `dict` state.** atomr-agents' state is a typed
  schema. Declare channels up front; `state.write("foo", ...)` to
  an unknown key errors.
- **Expecting Python-style runtime config (`configurable`).**
  Build-time wiring is the default. For runtime model swaps, wrap
  with `with_fallbacks`; for prompt swaps, use a middleware's
  `dynamic_prompt`.
- **Treating retrievers as `Runnable`s.** They implement `Retriever`,
  not `Callable` — wrap in a tool struct (see the
  `atomr-agents-rag` skill) to expose to the agent.
- **Mixing the legacy `WorkflowRunner` with channelled state.**
  The legacy runner predates `StateSchema`; use `StatefulRunner`
  (or `Interruptible`) for any new code.
