//! AI suggestion: lets the `claude` CLI pick the matching save folders from the scan results.
//!
//! The candidates from `scan.rs` are put into a prompt together with knowledge hints about
//! emulator folder layouts. The answer must be JSON; any path that wasn't in our candidate list
//! is thrown away, so the AI can only choose, never invent paths.
//!
//! Optional: if `claude` isn't installed, `ai_pick` returns an empty suggestion.
//! Used by `main.rs` (`scan_game`).

use super::scan::Cand;
use crate::adb;
use crate::util;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Environment variable to choose a different model; default is `DEFAULT_MODEL`.
const MODEL_ENV_VAR: &str = "THOR_SAVE_MODEL";
const DEFAULT_MODEL: &str = "sonnet";

/// The AI gets this long to answer before we give up.
const AI_TIMEOUT: Duration = Duration::from_secs(240);

/// The AI's choice per side, each with a short reason. `None` = no suggestion.
#[derive(Serialize, Deserialize, Default, Clone)]
pub struct AiPick {
    pub pc: Option<String>,
    pub pc_reason: Option<String>,
    pub thor: Option<String>,
    pub thor_reason: Option<String>,
}

/// Asks the `claude` CLI which candidate folders hold the saves of `game`.
///
/// Returns an empty `AiPick` if claude is missing, times out, or answers nonsense.
pub fn ai_pick(game: &str, pc: &[Cand], thor: &[Cand]) -> AiPick {
    // `let Some(x) = ... else { return ... };` unpacks the value or leaves the function.
    let Some(claude) = util::which("claude") else { return AiPick::default() };
    let model = std::env::var(MODEL_ENV_VAR).unwrap_or_else(|_| DEFAULT_MODEL.into());
    let prompt = build_prompt(game, pc, thor);

    let mut command = adb::cmd(claude);
    command.args(["-p", "--model", &model, "--tools", "", "--no-session-persistence"]);
    let Some(reply) = util::run_with_timeout(command, &prompt, AI_TIMEOUT) else { return AiPick::default() };
    parse_reply(&reply, pc, thor)
}

/// One line per candidate: `"<when> | <n> Dateien | <path> <note>"`.
/// (The German words are part of the prompt only, the user never sees them.)
fn format_candidates(candidates: &[Cand]) -> String {
    if candidates.is_empty() {
        return "(keine Kandidaten)".to_string();
    }
    let lines: Vec<String> = candidates
        .iter()
        .map(|cand| {
            let line = format!("{} | {} Dateien | {} {}", cand.when, cand.files, cand.path, cand.note);
            line.trim_end().to_string()
        })
        .collect();
    lines.join("\n")
}

/// Name of the PC's operating system for the prompt.
fn os_name() -> &'static str {
    if cfg!(windows) {
        "Windows"
    } else if cfg!(target_os = "macos") {
        "macOS"
    } else {
        "Linux"
    }
}

/// The full prompt with instructions, emulator layout hints and both candidate lists.
fn build_prompt(game: &str, pc: &[Cand], thor: &[Cand]) -> String {
    let os = os_name();
    // `r#"..."#` is a raw string (like `R"(...)"` in C++): no escaping needed for quotes.
    // `{{` / `}}` are literal braces inside `format!`.
    format!(
        r#"I want to sync the save files of "{game}" between my PC ({os}) and my
AYN Thor (Android handheld). Below are folder candidates from both devices
(last modified | number of files | path). On each side, pick the folder that directly
contains the saves of exactly this game.

Important:
- Both folders must match each other (same level/structure), because the contents are
  copied 1:1.
- Switch emulators (Eden, Yuzu, Citron, Sudachi, Ryujinx etc.) store saves under
  nand/user/save/0000000000000000/<user ID>/<TITLE ID>. The user ID differs on every
  device – pick the title ID folder itself. Use your knowledge of Switch title IDs
  (e.g. 0100F2C0115B6000 = Tears of the Kingdom). With several users, pick the most recently changed.
- 3DS emulators (Azahar, Citra, Lime3DS) store saves under
  sdmc/Nintendo 3DS/<id0>/<id1>/title/00040000/<title low, 8 hex>/data/00000001 – pick the
  00000001 folder. Use your knowledge of 3DS title IDs (e.g. Smash Bros. for 3DS:
  000ee000 = EU, 000edf00 = US, 000b8b00 = JP).
- For PS2/PS1/GameCube/Wii/PSP etc., pick the emulator's memory card / save folder accordingly.
- If the game isn't on one of the devices at all, return null for that side.
- On the Thor, paths may be inside a private app folder (/data/data/<package>/…),
  e.g. for native ports like bbport (Bloodborne). That's fine.
- NO ROM/install folders, caches, shader caches, logs or mods.
- Only return paths exactly as they appear in the list. If nothing fits: null.

Reply ONLY with JSON, no explanation around it:
{{"pc": "<path or null>", "pc_reason": "<short, English>", "thor": "<path or null>", "thor_reason": "<short, English>"}}

=== PC ===
{}

=== Thor ===
{}
"#,
        format_candidates(pc),
        format_candidates(thor)
    )
}

/// Extracts the JSON object from the reply (from the first `{` to the last `}`) and drops any
/// path that isn't one of our candidates.
fn parse_reply(reply: &str, pc: &[Cand], thor: &[Cand]) -> AiPick {
    let (Some(start), Some(end)) = (reply.find('{'), reply.rfind('}')) else { return AiPick::default() };
    let mut pick: AiPick = serde_json::from_str(&reply[start..=end]).unwrap_or_default();
    if !pc.iter().any(|cand| Some(&cand.path) == pick.pc.as_ref()) {
        pick.pc = None;
    }
    if !thor.iter().any(|cand| Some(&cand.path) == pick.thor.as_ref()) {
        pick.thor = None;
    }
    pick
}
