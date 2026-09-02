//! Optional local semantic search.
//!
//! Off by default and compiled out unless the `model-embeddings` feature is on (D8).
//! Lexical FTS5 answers most questions; the model earns its place only on the ones
//! where you remember what a plan was *about* but not a word it used.
//!
//! Nothing leaves the machine: the model is an ONNX file downloaded once into a shared
//! cache and then run locally by `fastembed`.

#[cfg(feature = "model-embeddings")]
use std::path::{Path, PathBuf};
#[cfg(feature = "model-embeddings")]
use std::sync::Mutex;

use crate::error::{Error, Result};

/// Produces dense vectors for plan text and queries. Behind a trait so search never
/// depends on a specific model, and so tests can substitute a deterministic one.
pub trait Embedder: Send + Sync {
    fn dims(&self) -> usize;
    /// Model identity, stored alongside each vector. Changing model invalidates the
    /// old vectors rather than silently mixing two vector spaces.
    fn id(&self) -> &str;
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>>;

    fn embed_one(&self, text: &str) -> Result<Vec<f32>> {
        let mut out = self.embed(&[text])?;
        out.pop()
            .ok_or_else(|| Error::invalid("embedder returned no vector"))
    }
}

/// Whether this build can embed at all.
pub const fn available() -> bool {
    cfg!(feature = "model-embeddings")
}

pub const DEFAULT_MODEL: &str = "bge-small-en-v1.5";

/// Build the configured embedder, or explain why this build cannot.
#[cfg(feature = "model-embeddings")]
pub fn build(model: &str, local_dir: Option<&Path>) -> Result<Box<dyn Embedder>> {
    match local_dir {
        Some(dir) => Ok(Box::new(LocalModel::from_dir(dir)?)),
        None => Ok(Box::new(LocalModel::cached(model)?)),
    }
}

#[cfg(not(feature = "model-embeddings"))]
pub fn build(_model: &str, _local_dir: Option<&std::path::Path>) -> Result<Box<dyn Embedder>> {
    Err(Error::invalid(
        "this build has no embedding model - reinstall with \
         `cargo install --git https://github.com/zottiben/ai-planner ai-planner --locked \
         --features model-embeddings`",
    ))
}

/// A local ONNX sentence embedder. No API keys, no network at query time.
#[cfg(feature = "model-embeddings")]
pub struct LocalModel {
    inner: Mutex<fastembed::TextEmbedding>,
    dims: usize,
    id: String,
}

#[cfg(feature = "model-embeddings")]
impl LocalModel {
    /// Fetch the model into the shared cache on first use, then load it from disk.
    ///
    /// The files are fetched with `curl`, not with the crate's own HTTP client, so the
    /// download uses the **system certificate store**. The bundled-roots TLS stack
    /// inside `hf-hub` fails outright behind a TLS-intercepting corporate proxy and
    /// ignores `SSL_CERT_FILE`, which would make this feature unusable on exactly the
    /// machines it is being built for.
    pub fn cached(name: &str) -> Result<Self> {
        let (_, dims, id) = resolve(name);
        let dir = cache_dir().join(id);
        if !model_present(&dir) {
            fetch_model(id, &dir)?;
        }
        let mut model = LocalModel::from_dir(&dir)?;
        model.id = id.to_string();
        if model.dims != dims {
            return Err(Error::invalid(format!(
                "{id} produced {}-dimension vectors, expected {dims}",
                model.dims
            )));
        }
        Ok(model)
    }

    /// Load a pre-downloaded model directory - for offline machines and for anyone
    /// behind a proxy that will not serve the model host. Dimensionality is probed
    /// from the model rather than trusted from a flag.
    pub fn from_dir(dir: &Path) -> Result<Self> {
        let read = |name: &str| -> Result<Vec<u8>> {
            let path = dir.join(name);
            std::fs::read(&path)
                .map_err(|e| Error::invalid(format!("reading {}: {e}", path.display())))
        };
        let model = fastembed::UserDefinedEmbeddingModel::new(
            read("model.onnx")?,
            fastembed::TokenizerFiles {
                tokenizer_file: read("tokenizer.json")?,
                config_file: read("config.json")?,
                special_tokens_map_file: read("special_tokens_map.json")?,
                tokenizer_config_file: read("tokenizer_config.json")?,
            },
        )
        .with_pooling(fastembed::Pooling::Cls);

        let mut inner = fastembed::TextEmbedding::try_new_from_user_defined(
            model,
            fastembed::InitOptionsUserDefined::default(),
        )
        .map_err(|e| Error::invalid(format!("loading model from {}: {e:#}", dir.display())))?;
        let dims = inner
            .embed(["dimension probe"], None)
            .map_err(|e| Error::invalid(e.to_string()))?
            .first()
            .map(|v| v.len())
            .ok_or_else(|| Error::invalid("model returned no vector for the probe"))?;

        let id = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("local")
            .to_string();
        Ok(LocalModel {
            inner: Mutex::new(inner),
            dims,
            id: format!("local:{id}"),
        })
    }
}

#[cfg(feature = "model-embeddings")]
impl Embedder for LocalModel {
    fn dims(&self) -> usize {
        self.dims
    }

    fn id(&self) -> &str {
        &self.id
    }

    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let mut model = self
            .inner
            .lock()
            .map_err(|e| Error::invalid(format!("embedder mutex poisoned: {e}")))?;
        model
            .embed(texts, None)
            .map_err(|e| Error::invalid(e.to_string()))
    }
}

/// One cache for the whole machine, so the model is downloaded once rather than once
/// per repo - matching how the database itself is shared (D1).
#[cfg(feature = "model-embeddings")]
pub fn cache_dir() -> PathBuf {
    std::env::var_os("AI_PLANNER_MODEL_CACHE")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache/ai-planner/fastembed"))
        })
        .unwrap_or_else(|| PathBuf::from(".ai-planner-model-cache"))
}

/// `(HuggingFace repo, dims, our id)`. The repos are the ONNX exports; the ids double
/// as cache directory names and as the model identity stored with every vector.
#[cfg(feature = "model-embeddings")]
fn resolve(name: &str) -> (&'static str, usize, &'static str) {
    let key = name.to_ascii_lowercase();
    if key.contains("minilm") {
        ("Xenova/all-MiniLM-L6-v2", 384, "all-minilm-l6-v2")
    } else if key.contains("bge-base") {
        ("Xenova/bge-base-en-v1.5", 768, "bge-base-en-v1.5")
    } else if key.contains("bge-large") {
        ("Xenova/bge-large-en-v1.5", 1024, "bge-large-en-v1.5")
    } else {
        ("Xenova/bge-small-en-v1.5", 384, "bge-small-en-v1.5")
    }
}

#[cfg(feature = "model-embeddings")]
const MODEL_FILES: [&str; 4] = [
    "tokenizer.json",
    "config.json",
    "special_tokens_map.json",
    "tokenizer_config.json",
];

#[cfg(feature = "model-embeddings")]
fn model_present(dir: &Path) -> bool {
    dir.join("model.onnx").is_file() && MODEL_FILES.iter().all(|f| dir.join(f).is_file())
}

#[cfg(feature = "model-embeddings")]
fn fetch_model(id: &str, dir: &Path) -> Result<()> {
    let (repo, _, _) = resolve(id);
    std::fs::create_dir_all(dir)
        .map_err(|e| Error::invalid(format!("creating {}: {e}", dir.display())))?;

    // Download beside the target and rename, so an interrupted fetch never leaves a
    // truncated model that looks cached.
    for (remote, local) in MODEL_FILES
        .iter()
        .map(|f| (f.to_string(), f.to_string()))
        .chain(std::iter::once((
            "onnx/model.onnx".to_string(),
            "model.onnx".to_string(),
        )))
    {
        let dest = dir.join(&local);
        if dest.is_file() {
            continue;
        }
        let url = format!("https://huggingface.co/{repo}/resolve/main/{remote}");
        let tmp = dir.join(format!("{local}.part"));
        // Only the weights are big enough to be worth a progress bar; the four json
        // files together are under 2 KB and their bars are just noise.
        let big = local.ends_with(".onnx");
        if big {
            eprintln!("fetching {id} weights from huggingface.co/{repo}");
        }
        download(&url, &tmp, big)?;
        std::fs::rename(&tmp, &dest)
            .map_err(|e| Error::invalid(format!("finishing {}: {e}", dest.display())))?;
    }
    Ok(())
}

#[cfg(feature = "model-embeddings")]
fn download(url: &str, dest: &Path, show_progress: bool) -> Result<()> {
    let (program, args): (&str, Vec<String>) = if which("curl") {
        (
            "curl",
            vec![
                if show_progress { "-fSL" } else { "-fsSL" }.into(),
                "--progress-bar".into(),
                "--retry".into(),
                "2".into(),
                "-o".into(),
                dest.to_string_lossy().into_owned(),
                url.to_string(),
            ],
        )
    } else if which("wget") {
        (
            "wget",
            vec![
                if show_progress {
                    "--show-progress"
                } else {
                    "-q"
                }
                .into(),
                "-q".into(),
                "-O".into(),
                dest.to_string_lossy().into_owned(),
                url.to_string(),
            ],
        )
    } else {
        return Err(Error::invalid(
            "curl or wget is needed to fetch the model - or pass --model-dir with a \
             pre-downloaded one",
        ));
    };

    let status = std::process::Command::new(program)
        .args(&args)
        .status()
        .map_err(|e| Error::invalid(format!("running {program}: {e}")))?;
    if !status.success() {
        let _ = std::fs::remove_file(dest);
        return Err(Error::invalid(format!("{program} could not fetch {url}")));
    }
    Ok(())
}

#[cfg(feature = "model-embeddings")]
fn which(program: &str) -> bool {
    std::process::Command::new(program)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Little-endian f32s. Portable across machines that share the database file.
pub fn to_blob(vector: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(vector.len() * 4);
    for x in vector {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}

pub fn from_blob(blob: &[u8]) -> Vec<f32> {
    blob.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// Cosine similarity, guarding the zero-vector case rather than returning NaN.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    if na <= 0.0 || nb <= 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A deterministic stand-in so the search path can be tested without a model.
    pub struct HashEmbedder {
        pub dims: usize,
    }

    impl Embedder for HashEmbedder {
        fn dims(&self) -> usize {
            self.dims
        }

        fn id(&self) -> &str {
            "test:hash"
        }

        fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
            Ok(texts
                .iter()
                .map(|text| {
                    let mut v = vec![0.0f32; self.dims];
                    for token in text
                        .split(|c: char| !c.is_alphanumeric())
                        .filter(|t| !t.is_empty())
                    {
                        let mut h: u64 = 1469598103934665603;
                        for b in token.to_ascii_lowercase().bytes() {
                            h ^= b as u64;
                            h = h.wrapping_mul(1099511628211);
                        }
                        v[(h as usize) % self.dims] += 1.0;
                    }
                    v
                })
                .collect())
        }
    }

    #[test]
    fn a_vector_survives_the_round_trip_through_a_blob() {
        let v = vec![0.5f32, -1.25, 0.0, 3.125];
        assert_eq!(from_blob(&to_blob(&v)), v);
    }

    #[test]
    fn cosine_is_one_for_itself_and_zero_for_the_degenerate_cases() {
        let a = vec![1.0f32, 2.0, 3.0];
        assert!((cosine(&a, &a) - 1.0).abs() < 1e-6);
        assert_eq!(cosine(&a, &[0.0, 0.0, 0.0]), 0.0);
        // Mismatched dimensions mean two different models; never pretend they compare.
        assert_eq!(cosine(&a, &[1.0, 2.0]), 0.0);
    }

    #[test]
    fn similar_text_scores_higher_than_unrelated_text() {
        let e = HashEmbedder { dims: 256 };
        let v = e
            .embed(&[
                "the date range picker panel",
                "a date range picker panel with two months",
                "gotenberg renders the invoice as a pdf",
            ])
            .unwrap();
        assert!(cosine(&v[0], &v[1]) > cosine(&v[0], &v[2]));
    }
}
