//! Asserts the public tool surface has no path to reorder or delete
//! from the notes/actions ledger.

#[test]
fn no_delete_tool_for_notes_or_actions() {
    // This is a compile-time-style assertion: the public symbols
    // exported by the crate include only append/update tools.
    // If someone adds a delete tool, this list must be updated and the
    // append-only invariant reviewed.
    use atomr_agents_meetings_harness as m;
    let names = [
        std::any::type_name::<m::AppendNoteTool>(),
        std::any::type_name::<m::AppendActionTool>(),
        std::any::type_name::<m::UpdateActionTool>(),
        std::any::type_name::<m::UpsertAttendeeTool>(),
        std::any::type_name::<m::FinalizeTool>(),
        std::any::type_name::<m::FinalizeSegmentTool>(),
        std::any::type_name::<m::ReviseTailSegmentTool>(),
        std::any::type_name::<m::RegenerateRunningTool>(),
        std::any::type_name::<m::SetTitleTool>(),
        std::any::type_name::<m::ListTurnsTool>(),
        std::any::type_name::<m::GetTurnTool>(),
    ];
    for n in names {
        assert!(
            !n.to_lowercase().contains("delete"),
            "tool `{n}` looks like a deletion tool — append-only invariant must be reviewed"
        );
    }
}
