//! `voice_input_skill` — a packaged Skill + tool that opts an agent
//! into "user might be speaking, transcribe their attachments"
//! mode.

use atomr_agents_core::{SkillId, ToolId};
use atomr_agents_skill::Skill;
use atomr_agents_stt_core::DynSpeechToText;
use atomr_agents_tool::DynTool;
use std::sync::Arc;

use crate::tool::TranscribeTool;

const VOICE_INSTRUCTION: &str = "\
The user may attach audio (voice messages, meeting recordings, dictation). \
When the user references an audio file or includes one in their turn, call the \
`transcribe_audio` tool with its path before answering. Treat the resulting \
transcript as if the user had typed it. Preserve speaker labels when present.";

/// Returns a [`Skill`] (instruction fragment + tool overlay) plus
/// the underlying [`TranscribeTool`] wrapped as `DynTool`. Register
/// the tool in your `ToolSet`, the Skill in your `SkillSet`, and
/// pick the Skill via any `SkillStrategy`.
pub fn voice_input_skill(stt: DynSpeechToText) -> (Skill, DynTool) {
    let tool: DynTool = Arc::new(TranscribeTool::new(stt));
    let skill = Skill {
        id: SkillId::from("voice_input"),
        name: "Voice input".into(),
        instruction_fragment: VOICE_INSTRUCTION.into(),
        tool_overlay: vec![ToolId::from("stt.transcribe")],
        memory_namespace: None,
        keywords: vec![
            "transcribe".into(),
            "audio".into(),
            "recording".into(),
            "voice memo".into(),
            "meeting".into(),
        ],
        priority: 6,
    };
    (skill, tool)
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_stt_core::MockSpeechToText;

    #[test]
    fn skill_has_tool_overlay() {
        let stt: DynSpeechToText = Arc::new(MockSpeechToText::new());
        let (skill, _tool) = voice_input_skill(stt);
        assert_eq!(skill.tool_overlay.len(), 1);
        assert_eq!(skill.tool_overlay[0].as_str(), "stt.transcribe");
        assert!(!skill.instruction_fragment.is_empty());
    }
}
