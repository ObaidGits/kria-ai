use std::path::Path;

/// Document preprocessing: extract text from common file formats.
pub struct DocumentProcessor;

impl DocumentProcessor {
    /// Extract text from a file based on extension.
    pub async fn extract_text(path: &Path) -> anyhow::Result<String> {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        match ext.as_str() {
            // Plain text / markup / data formats — direct UTF-8 read
            "txt" | "md" | "markdown" | "log" | "csv" | "json" | "toml"
            | "yaml" | "yml" | "xml" | "html" | "htm" => {
                Ok(tokio::fs::read_to_string(path).await?)
            }

            // Code files — all plain text
            "py" | "rs" | "ts" | "js" | "jsx" | "tsx" | "go" | "java"
            | "c" | "cpp" | "h" | "hpp" | "rb" | "php" | "kt" | "swift"
            | "lua" | "r" | "sh" | "bash" | "sql" => {
                Ok(tokio::fs::read_to_string(path).await?)
            }

            // Jupyter notebooks — extract source cells only
            "ipynb" => Self::extract_ipynb(path).await,

            // PDF via pdftotext (poppler), with raw-bytes fallback
            "pdf" => Self::extract_pdf(path).await,

            // DOCX via zip+XML text extraction (no pandoc required)
            "docx" | "doc" => Self::extract_docx(path).await,

            // XLSX / PPTX — pull text nodes from embedded XML
            "xlsx" | "xls" => Self::extract_xlsx(path).await,
            "pptx" | "ppt" => Self::extract_pptx(path).await,

            other => anyhow::bail!("unsupported document format: {other}"),
        }
    }

    // ── PDF ──────────────────────────────────────────────────────────────────

    async fn extract_pdf(path: &Path) -> anyhow::Result<String> {
        let p = path.to_path_buf();
        // Try pdftotext (poppler-utils) if available
        if let Ok(output) = tokio::process::Command::new("pdftotext")
            .args([p.to_string_lossy().as_ref(), "-"])
            .output()
            .await
        {
            if output.status.success() && !output.stdout.is_empty() {
                return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
            }
        }
        // Fallback: try pdf-extract via Python if available
        if let Ok(output) = tokio::process::Command::new("python3")
            .args(["-c", &format!(
                "import pdfminer.high_level, sys; print(pdfminer.high_level.extract_text('{}'))",
                p.to_string_lossy()
            )])
            .output()
            .await
        {
            if output.status.success() && !output.stdout.is_empty() {
                return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
            }
        }
        anyhow::bail!(
            "PDF text extraction failed — install poppler-utils (`sudo apt install poppler-utils`) for PDF support"
        )
    }

    // ── DOCX ─────────────────────────────────────────────────────────────────

    async fn extract_docx(path: &Path) -> anyhow::Result<String> {
        let p = path.to_path_buf();
        // DOCX is a ZIP; word/document.xml contains the text
        let text = tokio::task::spawn_blocking(move || -> anyhow::Result<String> {
            let file = std::fs::File::open(&p)?;
            let mut archive = zip::ZipArchive::new(file)?;

            let mut full_text = String::new();
            // Try word/document.xml first, then any .xml that looks like content
            let names: Vec<String> = (0..archive.len())
                .filter_map(|i| archive.by_index(i).ok().map(|f| f.name().to_string()))
                .collect();

            for name in &names {
                if name == "word/document.xml" || name.starts_with("word/document") {
                    let mut f = archive.by_name(name)?;
                    let mut xml = String::new();
                    std::io::Read::read_to_string(&mut f, &mut xml)?;
                    full_text.push_str(&strip_xml_tags(&xml));
                    full_text.push('\n');
                }
            }

            if full_text.trim().is_empty() {
                anyhow::bail!("no text content found in DOCX");
            }
            Ok(full_text)
        })
        .await??;
        Ok(text)
    }

    // ── XLSX ─────────────────────────────────────────────────────────────────

    async fn extract_xlsx(path: &Path) -> anyhow::Result<String> {
        let p = path.to_path_buf();
        let text = tokio::task::spawn_blocking(move || -> anyhow::Result<String> {
            let file = std::fs::File::open(&p)?;
            let mut archive = zip::ZipArchive::new(file)?;

            // xl/sharedStrings.xml has all cell text values
            let mut full_text = String::new();
            let targets = ["xl/sharedStrings.xml", "xl/worksheets/sheet1.xml"];
            let names: Vec<String> = (0..archive.len())
                .filter_map(|i| archive.by_index(i).ok().map(|f| f.name().to_string()))
                .collect();

            for name in &names {
                if targets.contains(&name.as_str()) || name.starts_with("xl/worksheets/") {
                    if let Ok(mut f) = archive.by_name(name) {
                        let mut xml = String::new();
                        std::io::Read::read_to_string(&mut f, &mut xml).ok();
                        full_text.push_str(&strip_xml_tags(&xml));
                        full_text.push('\n');
                    }
                }
            }

            if full_text.trim().is_empty() {
                anyhow::bail!("no text content found in XLSX");
            }
            Ok(full_text)
        })
        .await??;
        Ok(text)
    }

    // ── PPTX ─────────────────────────────────────────────────────────────────

    async fn extract_pptx(path: &Path) -> anyhow::Result<String> {
        let p = path.to_path_buf();
        let text = tokio::task::spawn_blocking(move || -> anyhow::Result<String> {
            let file = std::fs::File::open(&p)?;
            let mut archive = zip::ZipArchive::new(file)?;

            let mut full_text = String::new();
            let names: Vec<String> = (0..archive.len())
                .filter_map(|i| archive.by_index(i).ok().map(|f| f.name().to_string()))
                .collect();

            for name in &names {
                if name.starts_with("ppt/slides/slide") && name.ends_with(".xml") {
                    if let Ok(mut f) = archive.by_name(name) {
                        let mut xml = String::new();
                        std::io::Read::read_to_string(&mut f, &mut xml).ok();
                        full_text.push_str(&strip_xml_tags(&xml));
                        full_text.push('\n');
                    }
                }
            }

            if full_text.trim().is_empty() {
                anyhow::bail!("no text content found in PPTX");
            }
            Ok(full_text)
        })
        .await??;
        Ok(text)
    }

    // ── Jupyter Notebook ─────────────────────────────────────────────────────

    async fn extract_ipynb(path: &Path) -> anyhow::Result<String> {
        let raw = tokio::fs::read_to_string(path).await?;
        let nb: serde_json::Value = serde_json::from_str(&raw)?;

        let mut out = String::new();
        if let Some(cells) = nb["cells"].as_array() {
            for (i, cell) in cells.iter().enumerate() {
                let cell_type = cell["cell_type"].as_str().unwrap_or("code");
                let source = match &cell["source"] {
                    serde_json::Value::Array(lines) => lines
                        .iter()
                        .filter_map(|l| l.as_str())
                        .collect::<Vec<_>>()
                        .join(""),
                    serde_json::Value::String(s) => s.clone(),
                    _ => continue,
                };
                if !source.trim().is_empty() {
                    out.push_str(&format!("# Cell {} [{}]\n{}\n\n", i + 1, cell_type, source));
                }
            }
        }

        if out.is_empty() {
            anyhow::bail!("notebook has no source cells");
        }
        Ok(out)
    }
}

// ── XML tag stripper (used for DOCX/XLSX/PPTX ZIP-XML extraction) ─────────────

fn strip_xml_tags(xml: &str) -> String {
    let mut out = String::with_capacity(xml.len() / 2);
    let mut in_tag = false;
    for ch in xml.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    // Collapse whitespace
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}
