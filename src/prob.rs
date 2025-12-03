// SPDX-License-Identifier: GPL-3.0-or-later OR AGPL-3.0-or-later
// Copyright (C) 2025  Red Hat, Inc.

use serde_json::Value;

/// Calculate the response logprob from the token logprobs
///
/// If no logprobs are available, returns None
pub fn logprob(json: &Value) -> Option<f64> {
    // Check if logprobs exist in the response
    let logprobs = json
        .get("choices")
        .and_then(|c| c.as_array().and_then(|arr| arr.first()))
        .and_then(|c| c.get("logprobs"));

    // If no logprobs, return None
    let logprobs = logprobs?;

    // Extract content logprobs
    let content_logprobs = match logprobs.get("content") {
        Some(content) => content.as_array(),
        None => return None,
    };

    let content_logprobs = content_logprobs?;

    // If no content logprobs, return None
    if content_logprobs.is_empty() {
        return None;
    }

    // Find minimum log probability
    let mut min_logprob = f64::INFINITY;
    let mut min_logprob_pos: Option<usize> = None;
    for (i, token_logprob) in content_logprobs.iter().enumerate() {
        // Extract logprob value
        let logprob = match token_logprob.get("logprob") {
            Some(lp) => lp.as_f64(),
            None => continue,
        };

        if i == content_logprobs.len() - 1 {
            // Extract token value
            let token_value = match token_logprob.get("token") {
                Some(t) => t.as_str(),
                None => continue,
            };

            // Skip empty tokens
            if let Some(token) = token_value
                && token.is_empty()
            {
                continue;
            }
        }

        if let Some(lp) = logprob
            && lp < min_logprob
        {
            min_logprob = lp;
            min_logprob_pos = Some(i);
        }
    }

    // If no valid logprobs found, return None
    if min_logprob == f64::INFINITY {
        return None;
    }

    // Extract tokens from logprobs
    let tokens = logprobs
        .as_object()
        .and_then(|lp| lp.get("content"))
        .and_then(|c| c.as_array());

    // Call function with json and position of lowest logprob token
    print_logprob_diff(tokens, min_logprob_pos);

    Some(min_logprob)
}

fn print_logprob_diff(tokens: Option<&Vec<Value>>, min_logprob_pos: Option<usize>) {
    if let (Some(tokens), Some(pos)) = (tokens, min_logprob_pos) {
        // Extract tokens up to the position of the minimum logprob token
        let tokens_up_to_min: Vec<&Value> = tokens.iter().take(pos).collect();
        let mut concatenated_tokens = String::new();

        for token in tokens_up_to_min {
            if let Some(text) = token.get("token").and_then(|t| t.as_str()) {
                concatenated_tokens.push_str(text);
            }
        }

        let tokens_from_min: Vec<&Value> = tokens.iter().skip(pos).collect();
        let mut concatenated_rest = String::new();

        for token in tokens_from_min {
            if let Some(text) = token.get("token").and_then(|t| t.as_str()) {
                concatenated_rest.push_str(text);
            }
        }

        // Print the concatenated tokens
        log::trace!("Logprob:\n{}~~~{}", concatenated_tokens, concatenated_rest);
    }
}

pub fn logprob_to_prob(logprob: f64) -> f64 {
    //logprob.exp().min(1.0) * 100.
    1000000_f64.powf(logprob).clamp(0., 1.) * 100.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_logprob_with_logprobs() {
        let json_str = r#"{
            "choices": [
                {
                    "logprobs": {
                        "content": [
                            {
                                "logprob": 0.0
                            },
                            {
                                "logprob": -1.0
                            },
                            {
                                "logprob": -2.0,
				"token": " "
                            }
                        ]
                    }
                }
            ]
        }"#;

        let json: Value = serde_json::from_str(json_str).unwrap();
        let prob = logprob(&json);
        assert!(prob.is_some());
        assert!(
            prob.unwrap() == -2.0,
            "wrong prob: {} expected -2.0",
            prob.unwrap()
        );
    }

    #[test]
    fn test_logprob_no_logprobs() {
        let json_str = r#"{
            "choices": [
                {
                    "message": {
                        "content": "test"
                    }
                }
            ]
        }"#;

        let json: Value = serde_json::from_str(json_str).unwrap();
        let prob = logprob(&json);
        assert!(prob.is_none());
    }

    #[test]
    fn test_logprob_empty_logprobs() {
        let json_str = r#"{
            "choices": [
                {
                    "logprobs": {
                        "content": []
                    }
                }
            ]
        }"#;

        let json: Value = serde_json::from_str(json_str).unwrap();
        let prob = logprob(&json);
        assert!(prob.is_none());
    }
}

// Local Variables:
// rust-format-on-save: t
// End:
