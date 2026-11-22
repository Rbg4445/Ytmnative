use std::env;
use std::time::Instant;

use anyhow::{anyhow, Result};
use ytm_native::audio::probe_and_decode_sample;
use ytm_native::ytm::YtmClient;

#[tokio::main]
async fn main() -> Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .try_init();

    let query = env::args()
        .skip(1)
        .collect::<Vec<_>>()
        .join(" ");
    let query = if query.trim().is_empty() {
        "tarkan".to_string()
    } else {
        query
    };

    println!("=== YTM Native Smoke Test ===");
    println!("[1/4] İstemci başlatılıyor...");
    let client = YtmClient::new(None)?;

    let t0 = Instant::now();
    println!("[2/4] YouTube Music araması: \"{query}\"");
    let results = client.search_songs(&query).await?;
    if results.is_empty() {
        return Err(anyhow!("Arama sonucu boş döndü"));
    }
    println!(
        "  ✓ {} parça bulundu ({:.0} ms). İlk 3:",
        results.len(),
        t0.elapsed().as_millis()
    );
    for (i, t) in results.iter().take(3).enumerate() {
        println!(
            "    {}. {} — {} [{}] ({})",
            i + 1,
            t.artist,
            t.title,
            t.duration_string(),
            t.id
        );
    }

    let first = &results[0];

    let t1 = Instant::now();
    println!("[3/4] Akış çözülüyor: {} ({})...", first.title, first.id);
    let stream = client.resolve_stream(&first.id).await?;
    println!(
        "  ✓ Çözüldü ({:.0} ms) | kaynak: {} | {} | ~{} kbps | loudness: {:?} dB (gain: {:.2}x)",
        t1.elapsed().as_millis(),
        stream.source,
        stream.mime,
        stream.bitrate / 1000,
        stream.loudness_db,
        stream.normalization_gain()
    );

    let t2 = Instant::now();
    println!("[4/4] Progressive HTTP akış + Symphonia ses çözme testi (ilk 2.5 sn)...");
    let stream_clone = stream.clone();
    let stats = tokio::task::spawn_blocking(move || probe_and_decode_sample(&stream_clone, 2.5))
        .await??;
    println!(
        "  ✓ Çözüldü ({:.0} ms) | {} Hz, {} kanal | {} paket | {:.2} sn PCM | RMS: {:.4}",
        t2.elapsed().as_millis(),
        stats.sample_rate,
        stats.channels,
        stats.packets_decoded,
        stats.decoded_secs,
        stats.rms_level
    );

    // Bonus: test auto-radio queue
    match client.fetch_radio(&first.id).await {
        Ok(radio) => println!("  ✓ Otomatik radyo: {} benzer parça alındı.", radio.len()),
        Err(e) => println!("  ! Otomatik radyo uyarısı: {e}"),
    }

    println!("\n=== TÜM TESTLER BAŞARILI ===");
    Ok(())
}
