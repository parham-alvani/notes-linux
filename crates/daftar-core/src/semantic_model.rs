//! The model search by meaning runs: `minishlab/potion-multilingual-128M`,
//! a Model2Vec static embedding model of 101 languages, Persian among them,
//! run in the process (pure Rust, no server). Its files are not in the
//! binary: [download] fetches them, at a pinned revision and checked
//! against their SHA-256, into `$XDG_CACHE_HOME/daftar/models/`, when the
//! person asks (`daftar-cli semantic-index --download-model`).
//!
//! A static model is a table of a vector per token: a text's vector is the
//! mean of its tokens' rows, at length 1. The table, 500,353 tokens by 256,
//! is 512 MB of 32-bit floats in the file; it is read a row at a time into
//! 8-bit integers with a scale per row, so it holds 128 MB in memory.
//!
//! The tokens are found as the `tokenizers` crate's Unigram model finds
//! them (its normalizer and pre-tokenizer, then the same Viterbi search),
//! over a trie of our own: the crate's Unigram keeps a hash map at every
//! node of a trie of half a million pieces, 500 MB, which `model2vec-rs`
//! (Model2Vec's own Rust crate) keeps alive beside its faster trie. Here the
//! whole model holds about 200 MB, which an app open all day can carry.

use std::fs::{self, File};
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use rayon::prelude::*;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokenizers::{Normalizer, NormalizerWrapper, OffsetReferential, OffsetType, PreTokenizedString, PreTokenizer, PreTokenizerWrapper};

use crate::semantic::{Embedder, cache_dir};

/// The model's repository on Hugging Face, and the commit its files are taken from.
pub const REPOSITORY: &str = "minishlab/potion-multilingual-128M";
pub const REVISION: &str = "73908c3438cf03b6a01bcb9611d62b23d0726f08";

/// Each file: its name, size and SHA-256.
pub const FILES: [(&str, u64, &str); 3] = [
    ("config.json", 271, "595e4cab2093732efd5dbe084fd5c1826b5eea693b73b4c1fd971672867d2e54"),
    ("tokenizer.json", 18_616_131, "19f1909063da3cfe3bd83a782381f040dccea475f4816de11116444a73e1b6a1"),
    ("model.safetensors", 512_361_560, "14b5eb39cb4ce5666da8ad1f3dc6be4346e9b2d601c073302fa0a31bf7943397"),
];

/// The most tokens of a text that count, as Model2Vec reads them.
const MAX_TOKENS: usize = 512;

/// What the download weighs, in bytes.
pub fn download_size() -> u64 {
    FILES.iter().map(|(_, size, _)| size).sum()
}

/// The folder the model's files are in.
pub fn model_dir() -> PathBuf {
    cache_dir().join("models").join(format!("potion-multilingual-128M-{}", &REVISION[..8]))
}

/// The model's name and revision, as an index records it.
pub fn model_name() -> String {
    format!("{REPOSITORY}@{}", &REVISION[..8])
}

/// True when every file is in [model_dir] at its size: a file is put there
/// only once its checksum was right.
pub fn is_downloaded() -> bool {
    let folder = model_dir();
    FILES.iter().all(|(name, size, _)| fs::metadata(folder.join(name)).is_ok_and(|m| m.len() == *size))
}

/// What to say when the model is not there, to a person or an agent.
pub fn missing() -> String {
    format!(
        "search by meaning needs its model, which is not downloaded: run `daftar-cli semantic-index --download-model` once ({} MB, into {})",
        download_size().div_ceil(1_000_000),
        model_dir().display()
    )
}

/// Fetches the files that are missing into [model_dir], each into a
/// `.part` file renamed once its SHA-256 is right. [progress] hears the
/// bytes so far and the bytes in all. [agent] names the program to the server.
pub fn download(agent: &str, progress: &mut dyn FnMut(u64, u64)) -> Result<(), String> {
    let folder = model_dir();
    fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let client: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_recv_body(Some(Duration::from_mins(2)))
        .user_agent(agent)
        .build()
        .into();
    let total = download_size();
    let mut done = 0;
    for (name, size, sum) in FILES {
        let target = folder.join(name);
        if fs::metadata(&target).is_ok_and(|m| m.len() == size) {
            done += size;
            progress(done, total);
            continue;
        }
        let url = format!("https://huggingface.co/{REPOSITORY}/resolve/{REVISION}/{name}");
        let mut response = client.get(&url).call().map_err(|e| format!("{url}: {e}"))?;
        let mut body = response.body_mut().with_config().limit(size + 1).reader();
        let part = folder.join(format!("{name}.part"));
        let mut file = File::create(&part).map_err(|e| format!("{}: {e}", part.display()))?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0; 1 << 20];
        let mut read = 0;
        loop {
            let n = body.read(&mut buffer).map_err(|e| format!("{url}: {e}"))?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
            file.write_all(&buffer[..n]).map_err(|e| format!("{}: {e}", part.display()))?;
            read += n as u64;
            progress(done + read, total);
        }
        file.sync_all().map_err(|e| format!("{}: {e}", part.display()))?;
        let found = format!("{:x}", hasher.finalize());
        if read != size || found != sum {
            let _ = fs::remove_file(&part);
            return Err(format!("{url}: got {read} bytes with SHA-256 {found}, not {size} with {sum}"));
        }
        fs::rename(&part, &target).map_err(|e| format!("{}: {e}", target.display()))?;
        done += size;
    }
    Ok(())
}

/// A Model2Vec model, loaded.
pub struct Model2Vec {
    name: String,
    tokenizer: Unigram,
    dim: usize,
    /// Each token's row, as bytes, and the scale that makes them its vector.
    table: Vec<i8>,
    scales: Vec<f32>,
}

impl Model2Vec {
    /// The downloaded model; [missing] says why when it is not there.
    pub fn load() -> Result<Model2Vec, String> {
        if !is_downloaded() {
            return Err(missing());
        }
        Self::load_from(&model_dir(), &model_name())
    }

    /// The Model2Vec model in [folder] (`tokenizer.json` of a Unigram
    /// tokenizer and `model.safetensors` of one table), named [name] for the index.
    pub fn load_from(folder: &Path, name: &str) -> Result<Model2Vec, String> {
        let tokenizer = Unigram::load(&folder.join("tokenizer.json"))?;
        let (rows, dim, table, scales) = quantized(&folder.join("model.safetensors"))?;
        if rows < tokenizer.scores.len() {
            return Err(format!("{}: {rows} rows for {} tokens", folder.display(), tokenizer.scores.len()));
        }
        Ok(Model2Vec { name: name.to_string(), tokenizer, dim, table, scales })
    }

    pub fn dim(&self) -> usize {
        self.dim
    }

    /// The mean of the rows of [text]'s first tokens, at length 1; zeros
    /// for a text of no known token.
    fn vector(&self, text: &str) -> Vec<f32> {
        let mut sum = vec![0.0_f32; self.dim];
        let ids = self.tokenizer.ids(text);
        let mut count = 0;
        for &id in ids.iter().take(MAX_TOKENS).filter(|&&id| id != self.tokenizer.unk_id) {
            let id = id as usize;
            let scale = self.scales[id];
            for (total, &value) in sum.iter_mut().zip(&self.table[id * self.dim..(id + 1) * self.dim]) {
                *total += f32::from(value) * scale;
            }
            count += 1;
        }
        if count > 0 {
            crate::semantic::normalize(&mut sum);
        }
        sum
    }
}

impl Embedder for Model2Vec {
    fn model(&self) -> String {
        self.name.clone()
    }

    fn embed(&self, texts: &[String]) -> Vec<Vec<f32>> {
        texts.par_iter().map(|text| self.vector(text)).collect()
    }
}

/// The parts of a `tokenizer.json` a Unigram tokenizer needs.
#[derive(Deserialize)]
struct TokenizerFile {
    normalizer: Option<NormalizerWrapper>,
    pre_tokenizer: Option<PreTokenizerWrapper>,
    model: UnigramFile,
}

#[derive(Deserialize)]
struct UnigramFile {
    #[serde(rename = "type")]
    kind: String,
    unk_id: Option<usize>,
    vocab: Vec<(String, f64)>,
}

/// A Unigram tokenizer, as `tokenizers` runs one for ids alone.
struct Unigram {
    normalizer: Option<NormalizerWrapper>,
    pre_tokenizer: Option<PreTokenizerWrapper>,
    trie: Trie,
    scores: Vec<f64>,
    unk_id: u32,
    /// What an unknown character scores: 10 below the lowest piece, as `tokenizers` has it.
    unk_score: f64,
    /// The most characters of a text read: [MAX_TOKENS] tokens of the
    /// median piece's length, as Model2Vec cuts a text.
    max_chars: usize,
}

impl Unigram {
    fn load(file: &Path) -> Result<Unigram, String> {
        let failed = |e: &dyn std::fmt::Display| format!("{}: {e}", file.display());
        let reader = BufReader::with_capacity(1 << 20, File::open(file).map_err(|e| failed(&e))?);
        let parsed: TokenizerFile = serde_json::from_reader(reader).map_err(|e| failed(&e))?;
        if parsed.model.kind != "Unigram" {
            return Err(failed(&format!("a {} tokenizer, not a Unigram one", parsed.model.kind)));
        }
        let unk_id = parsed.model.unk_id.ok_or_else(|| failed(&"no unk_id"))? as u32;
        let scores: Vec<f64> = parsed.model.vocab.iter().map(|(_, score)| *score).collect();
        let lowest = scores.iter().copied().fold(f64::INFINITY, f64::min);
        let mut lengths: Vec<usize> = parsed.model.vocab.iter().map(|(piece, _)| piece.len()).collect();
        lengths.sort_unstable();
        let median = lengths.get(lengths.len() / 2).copied().unwrap_or(1).max(1);
        let pieces: Vec<String> = parsed.model.vocab.into_iter().map(|(piece, _)| piece).collect();
        Ok(Unigram {
            normalizer: parsed.normalizer,
            pre_tokenizer: parsed.pre_tokenizer,
            trie: Trie::build(&pieces),
            scores,
            unk_id,
            unk_score: lowest - 10.0,
            max_chars: MAX_TOKENS * median,
        })
    }

    /// The token ids of [text]: normalized and split as the tokenizer's
    /// file says, then each part by [viterbi].
    fn ids(&self, text: &str) -> Vec<u32> {
        let text = match text.char_indices().nth(self.max_chars) {
            Some((at, _)) => &text[..at],
            None => text,
        };
        let mut parts = PreTokenizedString::from(text);
        if let Some(normalizer) = &self.normalizer
            && parts.normalize(|n| normalizer.normalize(n)).is_err()
        {
            return Vec::new();
        }
        if let Some(pre_tokenizer) = &self.pre_tokenizer
            && pre_tokenizer.pre_tokenize(&mut parts).is_err()
        {
            return Vec::new();
        }
        let mut ids = Vec::new();
        for (part, _, _) in parts.get_splits(OffsetReferential::Normalized, OffsetType::None) {
            self.viterbi(part, &mut ids);
        }
        ids
    }

    /// The most likely pieces of [text], as `Unigram::encode_optimized`
    /// finds them: at each character, every piece that starts there,
    /// shortest first, taken over the best path so far to its end only when
    /// it scores more; a character no piece of its own covers is unknown.
    /// Unknown characters in a row are one unknown token.
    fn viterbi(&self, text: &str, ids: &mut Vec<u32>) {
        #[derive(Clone, Copy)]
        struct Best {
            score: f64,
            from: usize,
            id: u32,
            set: bool,
        }
        let bytes = text.as_bytes();
        if bytes.is_empty() {
            return;
        }
        let mut best = vec![Best { score: 0.0, from: 0, id: 0, set: false }; bytes.len() + 1];
        best[0].set = true;
        let relax = |slot: &mut Best, score: f64, from: usize, id: u32| {
            if !slot.set || score > slot.score {
                *slot = Best { score, from, id, set: true };
            }
        };
        let mut start = 0;
        while start < bytes.len() {
            let here = best[start].score;
            let width = text[start..].chars().next().map_or(1, char::len_utf8);
            let mut single = false;
            let mut node = 0;
            let mut end = start;
            while end < bytes.len() {
                let Some(next) = self.trie.child(node, bytes[end]) else { break };
                node = next;
                end += 1;
                if let Some(id) = self.trie.token(node) {
                    relax(&mut best[end], self.scores[id as usize] + here, start, id);
                    single |= end - start == width;
                }
            }
            if !single {
                relax(&mut best[start + width], self.unk_score + here, start, self.unk_id);
            }
            start += width;
        }
        let mark = ids.len();
        let mut end = bytes.len();
        let mut unknown = false;
        while end > 0 {
            let step = best[end];
            if step.id == self.unk_id {
                if !unknown {
                    ids.push(self.unk_id);
                }
                unknown = true;
            } else {
                ids.push(step.id);
                unknown = false;
            }
            end = step.from;
        }
        ids[mark..].reverse();
    }
}

/// A byte trie of the pieces: each node's edges side by side, sorted by
/// byte, so a step is a short binary search.
struct Trie {
    /// Each node's first edge, its edge count and its piece's id (or `u32::MAX`).
    nodes: Vec<(u32, u32, u32)>,
    edges: Vec<(u8, u32)>,
}

impl Trie {
    fn build(pieces: &[String]) -> Trie {
        // A piece written twice is its last id, as the crate's map has it.
        let mut order: Vec<u32> = (0..pieces.len() as u32).collect();
        order.sort_by(|&a, &b| pieces[a as usize].as_bytes().cmp(pieces[b as usize].as_bytes()).then(b.cmp(&a)));
        order.dedup_by(|a, b| pieces[*a as usize] == pieces[*b as usize]);
        let bytes = |i: usize| pieces[order[i] as usize].as_bytes();
        let mut trie = Trie { nodes: vec![(0, 0, u32::MAX)], edges: Vec::new() };
        // Each node with the run of [order] under it and its depth.
        let mut queue = std::collections::VecDeque::from([(0_usize, 0_usize, order.len(), 0_usize)]);
        while let Some((node, mut low, high, depth)) = queue.pop_front() {
            if low < high && bytes(low).len() == depth {
                trie.nodes[node].2 = order[low];
                low += 1;
            }
            let first = trie.edges.len() as u32;
            while low < high {
                let byte = bytes(low)[depth];
                let end = low + order[low..high].partition_point(|&i| pieces[i as usize].as_bytes()[depth] == byte);
                let child = trie.nodes.len();
                trie.nodes.push((0, 0, u32::MAX));
                trie.edges.push((byte, child as u32));
                queue.push_back((child, low, end, depth + 1));
                low = end;
            }
            trie.nodes[node].0 = first;
            trie.nodes[node].1 = trie.edges.len() as u32 - first;
        }
        trie
    }

    fn child(&self, node: usize, byte: u8) -> Option<usize> {
        let (first, count, _) = self.nodes[node];
        let edges = &self.edges[first as usize..(first + count) as usize];
        edges.binary_search_by_key(&byte, |&(b, _)| b).ok().map(|at| edges[at].1 as usize)
    }

    fn token(&self, node: usize) -> Option<u32> {
        Some(self.nodes[node].2).filter(|&id| id != u32::MAX)
    }
}

/// The table of a safetensors file of one table, `embeddings`, in 32-bit
/// floats, read a row at a time into 8-bit integers with a scale per row
/// (each row over its largest value): rows, columns, the table and the scales.
fn quantized(file: &Path) -> Result<(usize, usize, Vec<i8>, Vec<f32>), String> {
    let failed = |e: &dyn std::fmt::Display| format!("{}: {e}", file.display());
    let mut reader = BufReader::with_capacity(1 << 20, File::open(file).map_err(|e| failed(&e))?);
    let mut length = [0; 8];
    reader.read_exact(&mut length).map_err(|e| failed(&e))?;
    let length = u64::from_le_bytes(length);
    if length > 1 << 20 {
        return Err(failed(&"not a safetensors file"));
    }
    let mut header = vec![0; length as usize];
    reader.read_exact(&mut header).map_err(|e| failed(&e))?;
    let header: serde_json::Value = serde_json::from_slice(&header).map_err(|e| failed(&e))?;
    let tensor = &header["embeddings"];
    let shape: Vec<usize> =
        tensor["shape"].as_array().into_iter().flatten().filter_map(serde_json::Value::as_u64).map(|n| n as usize).collect();
    let offsets: Vec<u64> = tensor["data_offsets"].as_array().into_iter().flatten().filter_map(serde_json::Value::as_u64).collect();
    let unlike = || failed(&"not one table of 32-bit floats from its start, as a Model2Vec model is");
    let ([rows, cols], [0, end]) = (shape.as_slice(), offsets.as_slice()) else { return Err(unlike()) };
    let (rows, cols) = (*rows, *cols);
    if tensor["dtype"] != "F32" || cols == 0 || *end != (rows * cols * 4) as u64 {
        return Err(unlike());
    }
    let mut table = Vec::with_capacity(rows * cols);
    let mut scales = Vec::with_capacity(rows);
    let mut bytes = vec![0; cols * 4];
    let mut row = vec![0.0_f32; cols];
    for _ in 0..rows {
        reader.read_exact(&mut bytes).map_err(|e| failed(&e))?;
        for (value, b) in row.iter_mut().zip(bytes.as_chunks::<4>().0) {
            *value = f32::from_le_bytes(*b);
        }
        let largest = row.iter().fold(0.0_f32, |m, v| m.max(v.abs()));
        let scale = largest / 127.0;
        if scale > 0.0 {
            table.extend(row.iter().map(|v| (v / scale).round().clamp(-127.0, 127.0) as i8));
        } else {
            table.extend(std::iter::repeat_n(0, cols));
        }
        scales.push(scale);
    }
    Ok((rows, cols, table, scales))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A safetensors file of a 3 by 2 table, as `quantized` reads it.
    fn table(folder: &Path, values: &[f32], rows: usize, cols: usize) {
        let header = format!(r#"{{"embeddings":{{"dtype":"F32","shape":[{rows},{cols}],"data_offsets":[0,{}]}}}}"#, values.len() * 4);
        let mut bytes = (header.len() as u64).to_le_bytes().to_vec();
        bytes.extend_from_slice(header.as_bytes());
        bytes.extend(values.iter().flat_map(|v| v.to_le_bytes()));
        fs::write(folder.join("model.safetensors"), &bytes).unwrap();
    }

    #[test]
    fn the_table_is_read_a_row_at_a_time_into_bytes() {
        let folder = tempfile::tempdir().unwrap();
        table(folder.path(), &[1.0, -0.5, 0.0, 0.0, 0.25, 0.125], 3, 2);
        let (rows, cols, table, scales) = quantized(&folder.path().join("model.safetensors")).unwrap();
        assert_eq!((rows, cols), (3, 2));
        assert_eq!(table, [127, -64, 0, 0, 127, 64]);
        assert_eq!(scales, [1.0 / 127.0, 0.0, 0.25 / 127.0]);
        fs::write(folder.path().join("model.safetensors"), [4, 0, 0, 0, 0, 0, 0, 0, b'{', b'}', b' ', b' ']).unwrap();
        assert!(quantized(&folder.path().join("model.safetensors")).is_err());
    }

    /// A tokenizer of a few pieces, with Metaspace as the model's has it.
    fn tiny(folder: &Path) {
        let tokenizer = r#"{
            "normalizer": {"type": "Lowercase"},
            "pre_tokenizer": {"type": "Metaspace", "replacement": "▁", "prepend_scheme": "always", "split": false},
            "model": {"type": "Unigram", "unk_id": 0, "byte_fallback": false, "vocab": [
                ["<unk>", 0.0], ["▁", -3.0], ["▁ca", -2.0], ["▁cat", -1.0], ["s", -2.0], ["▁dog", -1.0], ["t", -3.0], ["a", -3.0], ["c", -3.0]
            ]}
        }"#;
        fs::write(folder.join("tokenizer.json"), tokenizer).unwrap();
    }

    #[test]
    fn a_text_is_cut_into_its_likeliest_pieces() {
        let folder = tempfile::tempdir().unwrap();
        tiny(folder.path());
        let unigram = Unigram::load(&folder.path().join("tokenizer.json")).unwrap();
        // ▁cat + s beats ▁ca + t + s; a space is ▁; ? is no piece, and two in a row are one unknown.
        assert_eq!(unigram.ids("Cats"), [3, 4]);
        assert_eq!(unigram.ids("cat dog"), [3, 5]);
        assert_eq!(unigram.ids("cat??"), [3, 0]);
        assert_eq!(unigram.ids(""), [] as [u32; 0]);
    }

    #[test]
    fn a_vector_is_the_mean_of_its_known_tokens() {
        let folder = tempfile::tempdir().unwrap();
        tiny(folder.path());
        // Rows: <unk>, ▁, ▁ca, ▁cat, s, ▁dog, t, a, c.
        let rows = [[9.0, 9.0], [0.0, 0.0], [0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [0.0, 2.0], [0.0, 0.0], [0.0, 0.0], [0.0, 0.0]];
        table(folder.path(), &rows.concat(), 9, 2);
        let model = Model2Vec::load_from(folder.path(), "tiny").unwrap();
        assert_eq!(model.dim(), 2);
        let found = model.embed(&["cats".into(), "cat??".into(), "??".into()]);
        let half = std::f32::consts::FRAC_1_SQRT_2;
        assert!((found[0][0] - half).abs() < 0.01 && (found[0][1] - half).abs() < 0.01, "{:?}", found[0]);
        // The unknown token is left out, not averaged in.
        assert!((found[1][0] - 1.0).abs() < 0.01 && found[1][1].abs() < 0.01, "{:?}", found[1]);
        assert_eq!(found[2], [0.0, 0.0]);
    }

    #[test]
    fn the_files_are_pinned() {
        assert_eq!(REVISION.len(), 40);
        assert!(FILES.iter().all(|(_, _, sum)| sum.len() == 64));
        assert_eq!(download_size(), 530_977_962);
        assert!(model_dir().starts_with(cache_dir().join("models")));
        assert!(missing().contains("--download-model") && missing().contains("531 MB"));
    }

    /// The real model, when it is downloaded: `cargo test -p daftar-core
    /// --features semantic -- --ignored`. Its tokens are the ones the
    /// `tokenizers` crate finds, and a Persian text is near its English.
    #[test]
    #[ignore = "needs the model, 531 MB"]
    fn the_model_tokenizes_as_tokenizers_does_and_meaning_crosses_languages() {
        let model = Model2Vec::load().unwrap();
        let reference = tokenizers::Tokenizer::from_file(model_dir().join("tokenizer.json")).unwrap();
        for text in [
            "The consumer lag of a message broker, on NATS JetStream (v2.11)!",
            "تاخیر مصرف‌کننده در صف پیام‌ها؛ ۱۲ ساعت",
            "  Mixed   فارسی and English: `ssh -p 7850 user@host` #tag [[Link|alias]] 😀 ✅ 2026-11-08",
            "naïve café — Ærøskøbing straße ﬁ ½",
        ] {
            let expected = reference.encode_fast(text, false).unwrap().get_ids().to_vec();
            assert_eq!(model.tokenizer.ids(text), expected, "{text}");
        }
        let texts = ["the consumer lag of a message broker", "تاخیر مصرف‌کننده در صف پیام", "a recipe for lemon cake"].map(String::from);
        let vectors = model.embed(&texts);
        let cosine = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
        assert!(cosine(&vectors[0], &vectors[1]) > cosine(&vectors[0], &vectors[2]));
    }
}
