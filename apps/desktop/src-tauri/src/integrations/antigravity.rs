use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const MAX_TRANSCRIPT_TAIL: u64 = 4 * 1024 * 1024;

pub(crate) fn record_stop(data: &Value, status: &str) {
    let diagnostic = json!({
        "recorded_at": chrono::Utc::now().to_rfc3339(),
        "status": status,
        "input_keys": data.as_object().map(|object| object.keys().collect::<Vec<_>>()),
        "executionNum": data["executionNum"],
        "terminationReason": data["terminationReason"],
        "fullyIdle": data["fullyIdle"],
        "has_error": data["error"].as_str().is_some_and(|error| !error.is_empty()),
        "has_transcript": data["transcriptPath"].as_str().is_some_and(|path| !path.is_empty()),
    });
    let path =
        super::permission_rules::home_dir().join(".agent-command-pilot/antigravity-last-stop.json");
    if let Err(error) = super::permission_rules::write_private(&path, &diagnostic) {
        eprintln!("Antigravity watch diagnostic: {error}");
    }
}

pub(crate) fn final_message(data: &Value) -> Result<Option<String>> {
    let Some(path) = data["transcriptPath"]
        .as_str()
        .filter(|path| !path.is_empty())
    else {
        return Ok(None);
    };
    read_final_message(Path::new(path))
}

fn read_final_message(path: &Path) -> Result<Option<String>> {
    let mut file = std::fs::File::open(path).context("无法打开 Antigravity 回复记录")?;
    let start = file.metadata()?.len().saturating_sub(MAX_TRANSCRIPT_TAIL);
    file.seek(SeekFrom::Start(start))?;
    let mut tail = Vec::new();
    file.take(MAX_TRANSCRIPT_TAIL).read_to_end(&mut tail)?;
    // Seeking can land in the middle of a UTF-8 character or JSON record.
    let tail = if start > 0 {
        let Some(newline) = tail.iter().position(|byte| *byte == b'\n') else {
            return Ok(None);
        };
        &tail[newline + 1..]
    } else {
        tail.as_slice()
    };
    final_text(std::str::from_utf8(tail)?)
}

fn final_text(transcript: &str) -> Result<Option<String>> {
    for line in transcript
        .lines()
        .rev()
        .filter(|line| !line.trim().is_empty())
    {
        // An unfinished or malformed last record must not expose an older reply.
        let step: Value = serde_json::from_str(line)?;
        if step["type"] == "USER_INPUT" {
            return Ok(None);
        }
        if step["source"] != "MODEL" || step["type"] != "PLANNER_RESPONSE" {
            continue;
        }
        if step["status"] != "DONE"
            || step["tool_calls"]
                .as_array()
                .is_some_and(|calls| !calls.is_empty())
        {
            return Ok(None);
        }
        return Ok(step["content"]
            .as_str()
            .filter(|text| !text.trim().is_empty())
            .map(str::to_owned));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn transcript(steps: &[Value]) -> String {
        steps
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn final_reply_excludes_thinking_tool_results_and_earlier_progress() {
        let reply = format!("完整回复：{}\n第二段", "测试".repeat(200));
        let steps = [
            json!({"type":"USER_INPUT","content":"问题"}),
            json!({"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","content":"正在检查","thinking":"私有推理","tool_calls":[{"name":"run_command"}]}),
            json!({"source":"MODEL","type":"GENERIC","status":"DONE","content":"终端输出"}),
            json!({"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","content":reply,"thinking":"私有推理"}),
        ];
        assert_eq!(final_text(&transcript(&steps)).unwrap(), Some(reply));
    }

    #[test]
    fn pending_turns_do_not_reuse_a_previous_reply() {
        let old = json!({"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","content":"上轮回复"});
        for latest in [
            json!({"type":"USER_INPUT","content":"新的问题"}),
            json!({"source":"MODEL","type":"PLANNER_RESPONSE","status":"RUNNING","content":"未完成"}),
            json!({"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","tool_calls":[{}],"content":"正在处理"}),
            json!({"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","thinking":"只有推理"}),
        ] {
            assert_eq!(
                final_text(&transcript(&[old.clone(), latest])).unwrap(),
                None
            );
        }
        assert!(final_text(&(transcript(&[old]) + "\n{unfinished")).is_err());
    }

    #[test]
    fn reads_only_a_bounded_tail_and_handles_missing_transcripts() {
        assert_eq!(final_message(&json!({})).unwrap(), None);
        let path = std::env::temp_dir().join(format!(
            "redmi-antigravity-{:016x}.jsonl",
            rand::random::<u64>()
        ));
        let reply = "实际回复💡";
        let record =
            json!({"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","content":reply});
        let content = format!(
            "{}\n{record}\n",
            "文".repeat(MAX_TRANSCRIPT_TAIL as usize / 3 + 10)
        );
        std::fs::write(&path, content).unwrap();
        let result = read_final_message(&path);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(result.unwrap(), Some(reply.into()));
        assert!(read_final_message(&path).is_err());
    }
}
