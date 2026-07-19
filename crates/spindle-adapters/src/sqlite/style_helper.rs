use anyhow::{Context, Result, anyhow};
use spindle_core::style::{StyleChunk, StyleCorpusMetrics};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub enum TextElement {
    Paragraph { text: String, word_count: usize },
    Heading { text: String },
    SceneBreak,
}

/// Helper to count words in a string.
pub fn count_words(s: &str) -> usize {
    s.split_whitespace().count()
}

/// Normalizes markdown content.
pub fn normalize_markdown(input: &str) -> String {
    // 1. Strip UTF-8 BOM
    let mut s = input.strip_prefix("\u{feff}").unwrap_or(input).to_string();

    // 2. Strip YAML frontmatter delimited by `---` at the start of the file.
    if s.starts_with("---")
        && let Some(next_idx) = s[3..].find("\n---")
    {
        let after_frontmatter = &s[3 + next_idx..];
        if let Some(rest) = after_frontmatter.strip_prefix("\n---") {
            s = rest.to_string();
        }
    }

    // 3. Strip HTML comments: `<!-- ... -->`
    while let Some(start_idx) = s.find("<!--") {
        if let Some(end_idx) = s[start_idx..].find("-->") {
            s.replace_range(start_idx..start_idx + end_idx + 3, "");
        } else {
            s.truncate(start_idx);
            break;
        }
    }

    // 4. Strip fenced code blocks
    let mut normalized_lines = Vec::new();
    let mut in_code_block = false;
    for line in s.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }
        if in_code_block {
            continue;
        }

        // 5. Convert ATX headings to blank-line separators
        if trimmed.starts_with('#') {
            let heading_level = trimmed.chars().take_while(|&c| c == '#').count();
            if heading_level > 0 && heading_level <= 6 && trimmed[heading_level..].starts_with(' ')
            {
                let heading_text = trimmed[heading_level..].trim().to_string();
                normalized_lines.push(format!("[HEADING: {}]", heading_text));
                continue;
            }
        }

        // 6. Treat scene break lines as scene breaks
        if trimmed == "***"
            || trimmed == "---"
            || trimmed == "___"
            || trimmed == "_ _ _"
            || trimmed == "* * *"
        {
            normalized_lines.push("[SCENE_BREAK]".to_string());
            continue;
        }

        normalized_lines.push(line.to_string());
    }

    let joined = normalized_lines.join("\n");

    // 7. Collapse runs of blank lines to maximum two blank lines (three newlines)
    let mut final_lines = Vec::new();
    let mut consecutive_blank = 0;
    for line in joined.lines() {
        if line.trim().is_empty() {
            consecutive_blank += 1;
            if consecutive_blank <= 2 {
                final_lines.push("".to_string());
            }
        } else {
            consecutive_blank = 0;
            final_lines.push(line.to_string());
        }
    }

    final_lines.join("\n")
}

/// Parses elements from normalized markdown text.
pub fn parse_elements(normalized_text: &str) -> Vec<TextElement> {
    let mut elements = Vec::new();
    let mut current_para = Vec::new();

    let flush_para = |current_para: &mut Vec<String>, elements: &mut Vec<TextElement>| {
        if !current_para.is_empty() {
            let text = current_para.join("\n");
            let word_count = count_words(&text);
            elements.push(TextElement::Paragraph { text, word_count });
            current_para.clear();
        }
    };

    for line in normalized_text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("[HEADING:") && trimmed.ends_with(']') {
            flush_para(&mut current_para, &mut elements);
            let heading_text = trimmed["[HEADING:".len()..trimmed.len() - 1]
                .trim()
                .to_string();
            elements.push(TextElement::Heading { text: heading_text });
        } else if trimmed == "[SCENE_BREAK]" {
            flush_para(&mut current_para, &mut elements);
            elements.push(TextElement::SceneBreak);
        } else if trimmed.is_empty() {
            flush_para(&mut current_para, &mut elements);
        } else {
            current_para.push(line.to_string());
        }
    }
    flush_para(&mut current_para, &mut elements);

    elements
}

/// Chunks normalized elements into style chunks.
pub fn chunk_elements(
    elements: &[TextElement],
    source_display_name: &str,
    total_corpus_words: usize,
) -> Vec<StyleChunk> {
    let mut chunks = Vec::new();
    let mut current_elements = Vec::new();
    let mut current_words = 0;
    let mut current_label = None;

    let mut flush = |current_elements: &mut Vec<TextElement>,
                     current_words: &mut usize,
                     current_label: &mut Option<String>| {
        if current_elements.is_empty() {
            return;
        }
        let mut text_parts = Vec::new();
        for el in current_elements.iter() {
            match el {
                TextElement::Paragraph { text, .. } => {
                    text_parts.push(text.clone());
                }
                TextElement::Heading { text } => {
                    text_parts.push(format!("# {}", text));
                }
                TextElement::SceneBreak => {
                    text_parts.push("***".to_string());
                }
            }
        }
        let chunk_text = text_parts.join("\n\n");
        let chunk_word_count = *current_words;

        let is_small_corpus = total_corpus_words < 1000;
        if chunk_word_count >= 150 || is_small_corpus {
            chunks.push(StyleChunk {
                text: chunk_text,
                word_count: chunk_word_count,
                label: current_label.clone(),
                source_display_name: source_display_name.to_string(),
            });
        }

        current_elements.clear();
        *current_words = 0;
    };

    for el in elements {
        match el {
            TextElement::Heading { text } => {
                if current_words >= 1200 {
                    flush(
                        &mut current_elements,
                        &mut current_words,
                        &mut current_label,
                    );
                }
                current_label = Some(text.clone());
                current_elements.push(el.clone());
            }
            TextElement::SceneBreak => {
                current_elements.push(el.clone());
                if current_words >= 1200 {
                    flush(
                        &mut current_elements,
                        &mut current_words,
                        &mut current_label,
                    );
                }
            }
            TextElement::Paragraph {
                text: _,
                word_count,
            } => {
                if current_words + word_count > 2500 && !current_elements.is_empty() {
                    flush(
                        &mut current_elements,
                        &mut current_words,
                        &mut current_label,
                    );
                }
                current_words += word_count;
                current_elements.push(el.clone());
            }
        }
    }
    flush(
        &mut current_elements,
        &mut current_words,
        &mut current_label,
    );

    chunks
}

/// Splits paragraph text into sentences.
pub fn split_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        current.push(c);
        if c == '.' || c == '?' || c == '!' {
            let mut j = i + 1;
            while j < chars.len()
                && (chars[j] == '"'
                    || chars[j] == '\''
                    || chars[j] == '”'
                    || chars[j] == '’'
                    || chars[j] == ')'
                    || chars[j] == ']')
            {
                current.push(chars[j]);
                j += 1;
            }
            if j == chars.len() || chars[j].is_whitespace() {
                let trimmed = current.trim();
                if !trimmed.is_empty() {
                    sentences.push(trimmed.to_string());
                }
                current.clear();
                i = j;
                continue;
            }
        }
        i += 1;
    }
    let trimmed = current.trim();
    if !trimmed.is_empty() {
        sentences.push(trimmed.to_string());
    }
    sentences
}

/// Counts words inside quotes in paragraph text.
pub fn count_dialogue_words(text: &str) -> usize {
    let mut inside = false;
    let mut dialogue_text = String::new();
    for c in text.chars() {
        if c == '"' || c == '“' || c == '”' {
            inside = !inside;
            dialogue_text.push(' ');
        } else if inside {
            dialogue_text.push(c);
        }
    }
    dialogue_text.split_whitespace().count()
}

/// Computes style metrics from parsed text elements.
pub fn compute_metrics(elements: &[TextElement]) -> StyleCorpusMetrics {
    let mut total_words = 0;
    let mut paragraphs = Vec::new();
    let mut sentences = Vec::new();

    for el in elements {
        if let TextElement::Paragraph { text, word_count } = el {
            total_words += word_count;
            paragraphs.push(text.clone());
            let para_sentences = split_sentences(text);
            sentences.extend(para_sentences);
        }
    }

    let paragraph_count = paragraphs.len();
    let sentence_count = sentences.len();

    // Sentence lengths
    let mut sentence_lengths: Vec<usize> = sentences
        .iter()
        .map(|s| s.split_whitespace().count())
        .filter(|&len| len > 0)
        .collect();
    sentence_lengths.sort();

    let average_sentence_words = if sentence_count > 0 {
        sentence_lengths.iter().sum::<usize>() as f64 / sentence_count as f64
    } else {
        0.0
    };

    let median_sentence_words = if sentence_lengths.is_empty() {
        0.0
    } else if sentence_lengths.len() % 2 == 1 {
        sentence_lengths[sentence_lengths.len() / 2] as f64
    } else {
        let mid = sentence_lengths.len() / 2;
        (sentence_lengths[mid - 1] + sentence_lengths[mid]) as f64 / 2.0
    };

    let p90_sentence_words = if sentence_lengths.is_empty() {
        0.0
    } else {
        let idx = (sentence_lengths.len() as f64 * 0.9).round() as usize;
        let idx = idx.min(sentence_lengths.len() - 1);
        sentence_lengths[idx] as f64
    };

    // Paragraph lengths
    let mut paragraph_lengths: Vec<usize> = paragraphs
        .iter()
        .map(|p| p.split_whitespace().count())
        .filter(|&len| len > 0)
        .collect();
    paragraph_lengths.sort();

    let average_paragraph_words = if paragraph_count > 0 {
        paragraph_lengths.iter().sum::<usize>() as f64 / paragraph_count as f64
    } else {
        0.0
    };

    let median_paragraph_words = if paragraph_lengths.is_empty() {
        0.0
    } else if paragraph_lengths.len() % 2 == 1 {
        paragraph_lengths[paragraph_lengths.len() / 2] as f64
    } else {
        let mid = paragraph_lengths.len() / 2;
        (paragraph_lengths[mid - 1] + paragraph_lengths[mid]) as f64 / 2.0
    };

    // Dialogue metrics
    let mut dialogue_lines = 0;
    let mut dialogue_words = 0;
    for p in &paragraphs {
        let is_dialogue = p.contains('"') || p.contains('“') || p.contains('”');
        if is_dialogue {
            dialogue_lines += 1;
            dialogue_words += count_dialogue_words(p);
        }
    }

    let dialogue_line_ratio = if paragraph_count > 0 {
        dialogue_lines as f64 / paragraph_count as f64
    } else {
        0.0
    };

    let dialogue_word_ratio = if total_words > 0 {
        dialogue_words as f64 / total_words as f64
    } else {
        0.0
    };

    // Concatenated text for string-wide counts
    let full_text = paragraphs.join("\n");

    let question_mark_rate_per_1k_words = if total_words > 0 {
        (full_text.chars().filter(|&c| c == '?').count() as f64 * 1000.0) / total_words as f64
    } else {
        0.0
    };

    let exclamation_rate_per_1k_words = if total_words > 0 {
        (full_text.chars().filter(|&c| c == '!').count() as f64 * 1000.0) / total_words as f64
    } else {
        0.0
    };

    let semicolon_rate_per_1k_words = if total_words > 0 {
        (full_text.chars().filter(|&c| c == ';').count() as f64 * 1000.0) / total_words as f64
    } else {
        0.0
    };

    let em_dash_rate_per_1k_words = if total_words > 0 {
        let count = full_text.matches('—').count() + full_text.matches("--").count();
        (count as f64 * 1000.0) / total_words as f64
    } else {
        0.0
    };

    let ellipsis_rate_per_1k_words = if total_words > 0 {
        let count = full_text.matches('…').count() + full_text.matches("...").count();
        (count as f64 * 1000.0) / total_words as f64
    } else {
        0.0
    };

    // Pronouns
    let lowercase_words: Vec<String> = full_text
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '\'')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();

    let first_person_pronouns: HashSet<&str> = [
        "i",
        "me",
        "my",
        "mine",
        "myself",
        "we",
        "us",
        "our",
        "ours",
        "ourselves",
    ]
    .iter()
    .cloned()
    .collect();

    let third_person_pronouns: HashSet<&str> = [
        "he",
        "him",
        "his",
        "himself",
        "she",
        "her",
        "hers",
        "herself",
        "it",
        "its",
        "itself",
        "they",
        "them",
        "their",
        "theirs",
        "themselves",
    ]
    .iter()
    .cloned()
    .collect();

    let mut first_person_count = 0;
    let mut third_person_count = 0;

    // Repeated markers map
    let mut word_freqs = BTreeMap::new();

    let stopwords: HashSet<&str> = [
        "the", "a", "an", "and", "or", "but", "of", "to", "in", "on", "at", "by", "for", "with",
        "about", "against", "between", "into", "through", "during", "before", "after", "above",
        "below", "from", "up", "down", "out", "off", "over", "under", "again", "further", "once",
        "is", "was", "were", "are", "be", "been", "being", "have", "has", "had", "having", "do",
        "does", "did", "doing", "will", "would", "shall", "should", "can", "could", "may", "might",
        "must", "that", "this", "these", "those", "who", "whom", "whose", "which", "what", "why",
        "how", "as", "if", "than", "because", "while", "until", "so", "i", "you", "he", "she",
        "it", "we", "they", "me", "him", "her", "us", "them", "my", "your", "his", "their", "our",
        "yours", "hers", "ours", "theirs", "its",
    ]
    .iter()
    .cloned()
    .collect();

    for w in &lowercase_words {
        if first_person_pronouns.contains(w.as_str()) {
            first_person_count += 1;
        }
        if third_person_pronouns.contains(w.as_str()) {
            third_person_count += 1;
        }
        if !stopwords.contains(w.as_str()) && w.chars().all(|c| c.is_alphabetic()) {
            *word_freqs.entry(w.clone()).or_insert(0) += 1;
        }
    }

    let first_person_pronoun_rate_per_1k_words = if total_words > 0 {
        (first_person_count as f64 * 1000.0) / total_words as f64
    } else {
        0.0
    };

    let third_person_pronoun_rate_per_1k_words = if total_words > 0 {
        (third_person_count as f64 * 1000.0) / total_words as f64
    } else {
        0.0
    };

    // Sort word frequencies to get top functional markers
    let mut freqs_vec: Vec<(String, usize)> = word_freqs.into_iter().collect();
    freqs_vec.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let top_functional_markers = freqs_vec.into_iter().take(15).map(|(w, _)| w).collect();

    StyleCorpusMetrics {
        average_sentence_words,
        median_sentence_words,
        p90_sentence_words,
        average_paragraph_words,
        median_paragraph_words,
        dialogue_line_ratio,
        dialogue_word_ratio,
        question_mark_rate_per_1k_words,
        exclamation_rate_per_1k_words,
        semicolon_rate_per_1k_words,
        em_dash_rate_per_1k_words,
        ellipsis_rate_per_1k_words,
        first_person_pronoun_rate_per_1k_words,
        third_person_pronoun_rate_per_1k_words,
        top_functional_markers,
    }
}

/// Helper to canonicalize a path and verify it is under the allowed roots.
pub fn resolve_and_verify_path(path_str: &str, allowed_roots: &[PathBuf]) -> Result<PathBuf> {
    let path = Path::new(path_str);
    // Note: canonicalize will return error if path doesn't exist, which is expected and correct!
    let canonical = fs::canonicalize(path)
        .with_context(|| format!("Failed to canonicalize path: {}", path_str))?;

    // Check allowed roots
    let mut allowed = false;
    for root in allowed_roots {
        if let Ok(root_canonical) = fs::canonicalize(root)
            && canonical.starts_with(root_canonical)
        {
            allowed = true;
            break;
        }
    }

    if !allowed {
        return Err(anyhow!("Path is outside allowed roots: {}", path_str));
    }

    Ok(canonical)
}

/// Helper to scan local paths for Markdown files recursively or directly.
pub fn collect_markdown_files(
    paths: &[String],
    recursive: bool,
    allowed_roots: &[PathBuf],
    max_files: usize,
    include_globs: Option<&[String]>,
    exclude_globs: Option<&[String]>,
) -> Result<Vec<PathBuf>> {
    let mut collected = Vec::new();
    let mut pending: Vec<PathBuf> = Vec::new();

    for path_str in paths {
        let canonical = resolve_and_verify_path(path_str, allowed_roots)?;
        pending.push(canonical);
    }

    let mut visited = HashSet::new();

    while let Some(path) = pending.pop() {
        if !visited.insert(path.clone()) {
            continue;
        }

        let metadata = fs::metadata(&path)?;
        if metadata.is_file() {
            if is_markdown_path(&path) && matches_glob_filters(&path, include_globs, exclude_globs)
            {
                collected.push(path);
            }
        } else if metadata.is_dir() {
            let is_explicit_start = paths.iter().any(|p_str| {
                if let Ok(c) = resolve_and_verify_path(p_str, allowed_roots) {
                    c == path
                } else {
                    false
                }
            });

            if recursive || is_explicit_start {
                for entry in fs::read_dir(&path)? {
                    let entry = entry?;
                    let entry_path = entry.path();
                    let name = entry_path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("");
                    if name.starts_with('.') {
                        continue;
                    }
                    if (recursive || entry_path.is_file())
                        && let Ok(c) = resolve_and_verify_path(
                            entry_path.to_str().unwrap_or(""),
                            allowed_roots,
                        )
                    {
                        pending.push(c);
                    }
                }
            }
        }
    }

    collected.sort();
    collected.truncate(max_files);

    Ok(collected)
}

fn is_markdown_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|ext| {
            let ext_lower = ext.to_ascii_lowercase();
            ext_lower == "md" || ext_lower == "markdown"
        })
        .unwrap_or(false)
}

fn matches_glob_filters(
    path: &Path,
    include_globs: Option<&[String]>,
    exclude_globs: Option<&[String]>,
) -> bool {
    let included = include_globs
        .filter(|globs| !globs.is_empty())
        .map(|globs| path_matches_any_glob(path, globs))
        .unwrap_or(true);
    let excluded = exclude_globs
        .filter(|globs| !globs.is_empty())
        .map(|globs| path_matches_any_glob(path, globs))
        .unwrap_or(false);
    included && !excluded
}

fn path_matches_any_glob(path: &Path, globs: &[String]) -> bool {
    let path_text = path.to_string_lossy().replace('\\', "/");
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    globs.iter().any(|glob| {
        let glob = glob.replace('\\', "/");
        wildcard_match(&glob, &path_text)
            || wildcard_match(&glob, file_name)
            || glob
                .strip_prefix("**/")
                .is_some_and(|suffix| wildcard_match(suffix, file_name))
    })
}

fn wildcard_match(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    let mut dp = vec![vec![false; text.len() + 1]; pattern.len() + 1];
    dp[0][0] = true;
    for i in 1..=pattern.len() {
        if pattern[i - 1] == '*' {
            dp[i][0] = dp[i - 1][0];
        }
    }
    for i in 1..=pattern.len() {
        for j in 1..=text.len() {
            dp[i][j] = match pattern[i - 1] {
                '*' => dp[i - 1][j] || dp[i][j - 1],
                '?' => dp[i - 1][j - 1],
                ch => dp[i - 1][j - 1] && ch == text[j - 1],
            };
        }
    }
    dp[pattern.len()][text.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn test_normalize_markdown() {
        let input = "\u{feff}---\ntitle: test\n---\n# Title\n\nSome text.\n\n```rust\nfn main() {}\n```\n<!-- comment -->\n\nMore text.\n***\nEnd.";
        let normalized = normalize_markdown(input);
        assert!(!normalized.contains("\u{feff}"));
        assert!(!normalized.contains("title: test"));
        assert!(!normalized.contains("fn main"));
        assert!(!normalized.contains("comment"));
        assert!(normalized.contains("[HEADING: Title]"));
        assert!(normalized.contains("[SCENE_BREAK]"));
        assert!(normalized.contains("Some text."));
        assert!(normalized.contains("More text."));
    }

    #[test]
    fn test_chunk_elements() {
        let elements = vec![
            TextElement::Heading {
                text: "Intro".to_string(),
            },
            TextElement::Paragraph {
                text: "Paragraph 1".to_string(),
                word_count: 500,
            },
            TextElement::Paragraph {
                text: "Paragraph 2".to_string(),
                word_count: 800,
            },
            TextElement::SceneBreak,
            TextElement::Paragraph {
                text: "Paragraph 3".to_string(),
                word_count: 1500,
            },
        ];
        let chunks = chunk_elements(&elements, "test", 2800);
        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].text.contains("Paragraph 1"));
        assert!(chunks[0].text.contains("Paragraph 2"));
        assert!(chunks[1].text.contains("Paragraph 3"));
    }

    #[test]
    fn test_compute_metrics() {
        let elements = vec![TextElement::Paragraph {
            text: "I went to the store. He was not there! \"Hello?\" she asked.".to_string(),
            word_count: 12,
        }];
        let metrics = compute_metrics(&elements);
        assert!((metrics.first_person_pronoun_rate_per_1k_words - 1000.0 / 12.0).abs() < 1e-5);
        assert!((metrics.third_person_pronoun_rate_per_1k_words - 2000.0 / 12.0).abs() < 1e-5);
        assert!(metrics.dialogue_line_ratio > 0.0);
        assert!(metrics.dialogue_word_ratio > 0.0);
        assert!(metrics.question_mark_rate_per_1k_words > 0.0);
        assert!(metrics.exclamation_rate_per_1k_words > 0.0);
    }

    #[test]
    fn test_path_safety() {
        let temp = tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let file_path = root.join("test.md");
        {
            let mut f = File::create(&file_path).unwrap();
            writeln!(f, "test").unwrap();
        }

        let resolved =
            resolve_and_verify_path(&file_path.to_string_lossy(), std::slice::from_ref(&root));
        assert!(resolved.is_ok());

        let outside_root = tempdir().unwrap();
        let outside_file = outside_root.path().join("outside.md");
        {
            let mut f = File::create(&outside_file).unwrap();
            writeln!(f, "outside").unwrap();
        }
        let resolved_outside =
            resolve_and_verify_path(&outside_file.to_string_lossy(), std::slice::from_ref(&root));
        assert!(resolved_outside.is_err());
    }

    #[test]
    fn test_collect_markdown_files_honors_globs() {
        let temp = tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let keep = root.join("keep.md");
        let skip = root.join("skip.md");
        let txt = root.join("note.txt");
        std::fs::write(&keep, "keep").unwrap();
        std::fs::write(&skip, "skip").unwrap();
        std::fs::write(&txt, "note").unwrap();

        let files = collect_markdown_files(
            &[root.to_string_lossy().to_string()],
            false,
            std::slice::from_ref(&root),
            100,
            Some(&["*.md".to_string()]),
            Some(&["skip.md".to_string()]),
        )
        .unwrap();

        assert_eq!(files, vec![std::fs::canonicalize(keep).unwrap()]);
    }
}
