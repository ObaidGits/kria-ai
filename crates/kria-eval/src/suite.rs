use crate::report::EvalCase;

pub fn load_suite(filepath: &str) -> Result<Vec<EvalCase>, String> {
    let content = std::fs::read_to_string(filepath)
        .map_err(|error| format!("failed to read suite file '{}': {}", filepath, error))?;

    let lines: Vec<&str> = content.lines().collect();
    let mut cases = Vec::new();
    let mut index = 0usize;

    while index < lines.len() {
        let line = lines[index].trim_start();
        let parsed = parse_prompt_line(line);
        let (case_id, prompt) = match parsed {
            Some(value) => value,
            None => {
                index += 1;
                continue;
            }
        };

        let mut expected_lines: Vec<String> = Vec::new();
        let mut found_expected = false;
        index += 1;

        while index < lines.len() {
            let raw_line = lines[index];
            let trimmed = raw_line.trim_start();

            if starts_test_id(trimmed) {
                break;
            }

            if !found_expected {
                if let Some(rest) = trimmed.strip_prefix("EXPECTED:") {
                    found_expected = true;
                    let first = rest.trim();
                    if !first.is_empty() {
                        expected_lines.push(first.to_string());
                    }
                }
                index += 1;
                continue;
            }

            if trimmed.is_empty() {
                break;
            }

            expected_lines.push(raw_line.trim_end().to_string());
            index += 1;
        }

        cases.push(EvalCase {
            id: case_id,
            prompt,
            expected_outcome: expected_lines.join("\n"),
            tags: Vec::new(),
            fixtures_ref: String::new(),
        });

        if index < lines.len() && lines[index].trim_start().is_empty() {
            index += 1;
        }
    }

    Ok(cases)
}

fn starts_test_id(line: &str) -> bool {
    if !line.starts_with('[') {
        return false;
    }
    let Some(close_idx) = line.find(']') else {
        return false;
    };
    close_idx > 1
}

fn parse_prompt_line(line: &str) -> Option<(String, String)> {
    if !line.starts_with('[') {
        return None;
    }

    let close_idx = line.find(']')?;
    let case_id = line[1..close_idx].trim();
    if case_id.is_empty() {
        return None;
    }

    let rest = line[close_idx + 1..].trim_start();
    if !rest.starts_with("Prompt") {
        return None;
    }

    let quote_start = rest.find('"')?;
    let quote_end = rest[quote_start + 1..].find('"')? + quote_start + 1;
    let prompt = rest[quote_start + 1..quote_end].trim();

    Some((case_id.to_string(), prompt.to_string()))
}
