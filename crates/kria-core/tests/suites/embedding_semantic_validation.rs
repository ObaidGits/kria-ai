//! Part 1 validation — the real ONNX embedding model (all-MiniLM-L6-v2 + its
//! real tokenizer) is active and produces semantically meaningful vectors, not
//! the deterministic hash fallback.
//!
//! Requires `~/.kria/models/embeddings/all-MiniLM-L6-v2.onnx` + `tokenizer.json`.

use kria_core::memory::embeddings::EmbeddingModel;

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

#[test]
fn onnx_model_active_and_semantically_clusters() {
    let model = EmbeddingModel::load(384).expect("load embedding model");

    assert!(
        model.is_onnx_loaded(),
        "REGRESSION: real ONNX model + tokenizer must be active (hash fallback is not acceptable in production). \
         Ensure ~/.kria/models/embeddings/all-MiniLM-L6-v2.onnx + tokenizer.json exist."
    );

    let emb = |s: &str| model.embed(s).expect("embed");

    // Intra-cluster (should be close) vs cross-cluster (should be farther).
    let calc = [
        "calculate an expression",
        "compute the result",
        "evaluate this math",
    ];
    let json = ["pretty print json", "format a json document", "minify json"];
    let hash = ["sha256 hash", "compute md5 digest", "hash this text"];

    let calc_v: Vec<_> = calc.iter().map(|s| emb(s)).collect();
    let json_v: Vec<_> = json.iter().map(|s| emb(s)).collect();
    let hash_v: Vec<_> = hash.iter().map(|s| emb(s)).collect();

    // Average intra-cluster similarity for calc.
    let intra_calc = (cosine(&calc_v[0], &calc_v[1])
        + cosine(&calc_v[0], &calc_v[2])
        + cosine(&calc_v[1], &calc_v[2]))
        / 3.0;
    let intra_json = (cosine(&json_v[0], &json_v[1])
        + cosine(&json_v[0], &json_v[2])
        + cosine(&json_v[1], &json_v[2]))
        / 3.0;
    let intra_hash = (cosine(&hash_v[0], &hash_v[1])
        + cosine(&hash_v[0], &hash_v[2])
        + cosine(&hash_v[1], &hash_v[2]))
        / 3.0;

    // Cross-cluster similarity (calc vs json).
    let cross = cosine(&calc_v[0], &json_v[0]);

    eprintln!(
        "[EMBED] intra_calc={intra_calc:.3} intra_json={intra_json:.3} intra_hash={intra_hash:.3} cross(calc,json)={cross:.3}"
    );

    // Intra-cluster similarity is strongly positive (≫ the ~0.0 cross-cluster);
    // 0.4 is a robust floor (real all-MiniLM clusters these at 0.49–0.73).
    assert!(
        intra_calc > 0.4,
        "calc cluster should be tight, got {intra_calc:.3}"
    );
    assert!(
        intra_json > 0.4,
        "json cluster should be tight, got {intra_json:.3}"
    );
    assert!(
        intra_hash > 0.4,
        "hash cluster should be tight, got {intra_hash:.3}"
    );
    assert!(
        intra_calc > cross && intra_json > cross,
        "intra-cluster similarity ({intra_calc:.3}/{intra_json:.3}) must exceed cross-cluster ({cross:.3})"
    );

    // A clearly-unrelated phrase must be far from all clusters.
    let unrelated = emb("water the office plants");
    let calc_sim = cosine(&unrelated, &calc_v[0]);
    eprintln!("[EMBED] cosine(unrelated, calc)={calc_sim:.3}");
    assert!(
        calc_sim < intra_calc,
        "unrelated phrase must be less similar than intra-cluster"
    );

    eprintln!("[PASS] ONNX embeddings active + semantic clustering verified");
}
