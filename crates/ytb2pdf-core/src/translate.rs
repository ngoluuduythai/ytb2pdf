//! Caption translation.
//!
//! [`Translator`] is a trait so the UI can offer "no translation", Gemini,
//! or a future local model without touching the pipeline. [`GeminiTranslator`]
//! batches cues to stay inside free-tier rate limits.

use anyhow::{anyhow, Context, Result};

pub trait Translator: Send + Sync {
    /// Translate `texts` (kept in order) into `target_lang` (e.g. `"vi"`).
    /// Returns one translated string per input.
    fn translate(&self, texts: &[String], target_lang: &str) -> Result<Vec<String>>;
}

/// Returns the input unchanged.
pub struct NoopTranslator;

impl Translator for NoopTranslator {
    fn translate(&self, texts: &[String], _target: &str) -> Result<Vec<String>> {
        Ok(texts.to_vec())
    }
}

/// Machine translation via the Gemini `generateContent` REST API.
/// Needs `GEMINI_API_KEY` (free key from aistudio.google.com).
pub struct GeminiTranslator {
    pub api_key: String,
    pub model: String,
    /// cues per request; 20 keeps prompts small and failures cheap
    pub batch_size: usize,
}

impl GeminiTranslator {
    pub fn from_env() -> Result<Self> {
        let api_key = std::env::var("GEMINI_API_KEY")
            .map_err(|_| anyhow!("GEMINI_API_KEY is not set"))?;
        let model =
            std::env::var("GEMINI_MODEL").unwrap_or_else(|_| "gemini-3.5-flash-lite".to_string());
        Ok(Self { api_key, model, batch_size: 20 })
    }

    fn translate_batch(&self, batch: &[String], target_lang: &str) -> Result<Vec<String>> {
        let numbered: Vec<String> = batch
            .iter()
            .enumerate()
            .map(|(i, t)| format!("[{}] {}", i + 1, t))
            .collect();
        let prompt = format!(
            "Translate the following numbered subtitle lines to {target_lang} (language code: {target_lang}). \
             Rules: return ONLY the translations, one per line, numbered exactly like the input, in the same order. \
             Do not add explanations, quotes, or extra lines. Keep timestamps out — there are none.\n\n{}",
            numbered.join("\n")
        );
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
            self.model, self.api_key
        );
        let body = serde_json::json!({
            "contents": [{ "parts": [{ "text": prompt }] }],
            "generationConfig": { "temperature": 0.2, "maxOutputTokens": 8192 }
        });
        let resp: serde_json::Value = ureq::post(&url)
            .send_json(&body)
            .map_err(|e| anyhow!("Gemini request failed: {e}"))?
            .into_json()
            .context("decoding Gemini response")?;
        let text = resp
            .get("candidates").and_then(|c| c.get(0))
            .and_then(|c| c.get("content")).and_then(|c| c.get("parts")).and_then(|p| p.get(0))
            .and_then(|p| p.get("text")).and_then(|t| t.as_str())
            .ok_or_else(|| anyhow!("unexpected Gemini response shape: {}",
                serde_json::to_string(&resp).unwrap_or_default().chars().take(300).collect::<String>()))?;
        let mut out: Vec<String> = text
            .lines()
            .map(|l| {
                // strip a leading "[12] " / "12. " / "12: " marker
                let l = l.trim();
                let l = l.strip_prefix('[').unwrap_or(l);
                let cut = l.find(|c: char| c == ']' || c == '.' || c == ':').map(|i| i + 1).unwrap_or(0);
                l[cut..].trim().to_string()
            })
            .filter(|l| !l.is_empty())
            .collect();
        // Be lenient: pad/truncate so the caller always gets 1:1 alignment.
        out.resize_with(batch.len(), || String::new());
        out.truncate(batch.len());
        for (i, t) in out.iter_mut().enumerate() {
            if t.is_empty() {
                *t = batch[i].clone(); // fall back to the original line
            }
        }
        Ok(out)
    }
}

impl Translator for GeminiTranslator {
    fn translate(&self, texts: &[String], target_lang: &str) -> Result<Vec<String>> {
        let mut out = Vec::with_capacity(texts.len());
        for chunk in texts.chunks(self.batch_size) {
            // gentle pacing for the free tier
            std::thread::sleep(std::time::Duration::from_millis(400));
            out.extend(self.translate_batch(chunk, target_lang)?);
        }
        Ok(out)
    }
}
