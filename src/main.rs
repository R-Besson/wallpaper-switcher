use anyhow::{anyhow, Context, Result};
use display_info::DisplayInfo;
use rand::seq::IndexedRandom;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::{fs, time};
use user_idle::UserIdle;

#[derive(Deserialize, Serialize)]
struct Config {
    api_key: String,
    delay: String,
    sources: Vec<String>,
}

#[derive(Deserialize)]
struct UnsplashPhoto {
    id: String,
    width: u32,
    height: u32,
    urls: UnsplashUrls,
}

#[derive(Deserialize)]
struct UnsplashUrls {
    raw: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let client = reqwest::Client::new();
    
    let image_path = dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("current_wallpaper.jpg");
    
    let config_path = std::env::current_exe()?
        .parent()
        .context("Failed to get executable directory")?
        .join("config.json");

    println!("--- Wallpaper Switcher Started ---");
    println!("Image path: {:?}", image_path);

    if !config_path.exists() {
        let default_config = Config {
            api_key: "YOUR_UNSPLASH_API_KEY".to_string(),
            delay: "1h".to_string(),
            sources: vec!["nature".to_string(), "space".to_string(), "minimalist".to_string()],
        };
        let json = serde_json::to_string_pretty(&default_config)?;
        fs::write(&config_path, json).await?;
        println!("Default config created at {:?}. Please edit it.", config_path);
        return Ok(());
    }

    loop {
        let config_str = fs::read_to_string(&config_path).await.context("Failed to read config.json")?;
        let config: Config = serde_json::from_str(&config_str).context("Failed to parse config.json")?;
        let duration = humantime::parse_duration(&config.delay).context("Invalid delay format")?;

        if config.api_key == "YOUR_UNSPLASH_API_KEY" {
            println!("Please update config.json with a valid API Key.");
            time::sleep(time::Duration::from_secs(10)).await;
            continue;
        }

		let idle_seconds = UserIdle::get_time().map(|i| i.as_seconds()).unwrap_or(0);
		if idle_seconds < 600 { 
			if let Err(e) = update(&client, &config, &image_path).await {
				eprintln!("Error occurred during update: {:#}", e);
			}
			
			println!("Sleeping for {:?}...\n", duration);
			time::sleep(duration).await;
		}

    }
}

async fn update(client: &reqwest::Client, config: &Config, image_path: &PathBuf) -> Result<()> {
    println!("--- Starting Update Cycle ---");

    let (max_w, max_h) = DisplayInfo::all()
        .unwrap_or_default()
        .iter()
        .map(|d| (d.width, d.height))
        .max()
        .unwrap_or((1920, 1080));

    println!("Target Resolution: {}x{}", max_w, max_h);

    let photo: UnsplashPhoto = loop {
        let query = config.sources.choose(&mut rand::rng()).context("Sources list is empty")?;
        println!("Selected Query: '{}'", query);
        
        let params = [("query", query.as_str()), ("orientation", "landscape")];

        let response = client
            .get("https://api.unsplash.com/photos/random")
            .query(&params)
            .header("Authorization", format!("Client-ID {}", config.api_key))
            .send()
            .await?;

        if !response.status().is_success() {
             println!("API Request failed: {}", response.status());
             time::sleep(time::Duration::from_secs(5)).await;
             continue;
        }

        let candidate: UnsplashPhoto = response.json().await?;
        println!("Candidate ID: {} ({}x{})", candidate.id, candidate.width, candidate.height);

        if candidate.width >= max_w && candidate.height >= max_h {
            break candidate;
        } else {
            println!("Image too small for screen. Retrying...");
            time::sleep(time::Duration::from_secs(2)).await; 
        }
    };

    println!("Downloading RAW full-resolution image...");
    let bytes = client
        .get(&photo.urls.raw)
        .send()
        .await?
        .bytes()
        .await?;
    
    println!("Download complete. Size: {:.2} MB", bytes.len() as f64 / 1_000_000.0);

    println!("Saving raw file directly to disk...");
    fs::write(image_path, &bytes).await.context("Failed to save wallpaper file")?;

    println!("Image saved to {:?}", image_path);

    wallpaper::set_from_path(image_path.to_str().unwrap()).map_err(|e| anyhow!(e.to_string()))?;
    wallpaper::set_mode(wallpaper::Mode::Crop).map_err(|e| anyhow!(e.to_string()))?;

    println!("Wallpaper updated successfully!");
    Ok(())
}