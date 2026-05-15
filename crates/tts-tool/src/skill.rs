//! `voice_response_skill` — bundles `SpeakTool` with `TranscribeTool`
//! into a Skill that opts an agent into voice-driven I/O.

use std::sync::Arc;

use atomr_agents_core::{SkillId, ToolId};
use atomr_agents_skill::Skill;
use atomr_agents_stt_core::DynSpeechToText;
use atomr_agents_stt_tool::TranscribeTool;
use atomr_agents_tool::DynTool;
use atomr_agents_tts_core::DynTextToSpeech;

use crate::tool::SpeakTool;

const VOICE_RESPONSE_INSTRUCTION: &str = "\
The user is interacting via voice. After understanding their request, call the \
`speak_text` tool with your reply so the user hears it. If the user has shared \
audio, call `transcribe_audio` first to understand what they said. Keep replies \
short — they will be vocalised.";

const SPEAK_INSTRUCTION: &str = "\
You can vocalise your reply by calling the `speak_text` tool with the text you \
want the user to hear. Reserve this for replies the user expects in voice.";

/// Bundles speak + transcribe into a single Skill.
pub fn voice_response_skill(stt: DynSpeechToText, tts: DynTextToSpeech) -> (Skill, Vec<DynTool>) {
    let speak: DynTool = Arc::new(SpeakTool::new(tts));
    let transcribe: DynTool = Arc::new(TranscribeTool::new(stt));
    let skill = Skill {
        id: SkillId::from("voice_response"),
        name: "Voice response".into(),
        instruction_fragment: VOICE_RESPONSE_INSTRUCTION.into(),
        tool_overlay: vec![ToolId::from("tts.speak"), ToolId::from("stt.transcribe")],
        memory_namespace: None,
        keywords: vec![
            "voice".into(),
            "speak".into(),
            "say".into(),
            "audio reply".into(),
            "vocalise".into(),
        ],
        priority: 7,
    };
    (skill, vec![speak, transcribe])
}

/// Speak-only skill (no inbound transcription).
pub fn voice_speak_skill(tts: DynTextToSpeech) -> (Skill, DynTool) {
    let tool: DynTool = Arc::new(SpeakTool::new(tts));
    let skill = Skill {
        id: SkillId::from("voice_speak"),
        name: "Speak reply".into(),
        instruction_fragment: SPEAK_INSTRUCTION.into(),
        tool_overlay: vec![ToolId::from("tts.speak")],
        memory_namespace: None,
        keywords: vec!["speak".into(), "say".into(), "vocalise".into()],
        priority: 5,
    };
    (skill, tool)
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_stt_core::MockSpeechToText;
    use atomr_agents_tts_core::MockTextToSpeech;

    #[test]
    fn voice_response_skill_has_two_tools() {
        let stt: DynSpeechToText = Arc::new(MockSpeechToText::new());
        let tts: DynTextToSpeech = Arc::new(MockTextToSpeech::default());
        let (skill, tools) = voice_response_skill(stt, tts);
        assert_eq!(tools.len(), 2);
        assert_eq!(skill.tool_overlay.len(), 2);
        assert!(!skill.instruction_fragment.is_empty());
    }

    #[test]
    fn voice_speak_skill_has_one_tool() {
        let tts: DynTextToSpeech = Arc::new(MockTextToSpeech::default());
        let (skill, _tool) = voice_speak_skill(tts);
        assert_eq!(skill.tool_overlay.len(), 1);
        assert_eq!(skill.tool_overlay[0].as_str(), "tts.speak");
    }
}
