#![windows_subsystem = "windows"]

use anyhow::{Context, Result, anyhow};
use display_info::DisplayInfo;
use rand::seq::IndexedRandom;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::{fs, time};
use user_idle::UserIdle;

use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tray_icon::{
	TrayIconBuilder,
	menu::{Menu, MenuEvent, MenuItem},
};

#[derive(Deserialize, Serialize)]
struct Config {
	api_key: String,
	delay: String,
	sources: Vec<String>,
}

#[derive(Deserialize)]
struct UnsplashPhoto {
	// id: String,
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
	let tray_menu = Menu::new();
	let quit_i: MenuItem = MenuItem::new("Quit Wallpaper Switcher", true, None);
	let _ = tray_menu.append_items(&[&quit_i]);

	let _tray_icon = TrayIconBuilder::new()
		.with_menu(Box::new(tray_menu))
		.with_tooltip("Wallpaper Switcher")
		.build()
		.unwrap();

	tokio::spawn(async move {
		if let Err(e) = run_wallpaper_service().await {
			eprintln!("Service Error: {}", e);
		}
	});

	let event_loop = EventLoopBuilder::new().build();
	let menu_channel = MenuEvent::receiver();

	event_loop.run(move |_event, _, control_flow| {
		*control_flow = ControlFlow::Wait;

		if let Ok(event) = menu_channel.try_recv() && event.id == quit_i.id() {
			*control_flow = ControlFlow::Exit;
		}
	});
}

async fn run_wallpaper_service() -> Result<()> {
	let client = reqwest::Client::new();
	let image_path = dirs::cache_dir()
		.unwrap_or_else(|| PathBuf::from("."))
		.join("current_wallpaper.jpg");

	let config_path = std::env::current_exe()?
		.parent()
		.context("Failed to get executable directory")?
		.join("config.json");

	if !config_path.exists() {
		let default_config = Config {
			api_key: "YOUR_UNSPLASH_API_KEY".to_string(),
			delay: "1h".to_string(),
			sources: vec![
				"nature".to_string(),
				"space".to_string(),
				"minimalist".to_string(),
			],
		};
		let json = serde_json::to_string_pretty(&default_config)?;
		fs::write(&config_path, json).await?;
		return Ok(());
	}

	loop {
		let config_str = fs::read_to_string(&config_path)
			.await
			.context("Failed to read config.json")?;
		let config: Config =
			serde_json::from_str(&config_str).context("Failed to parse config.json")?;
		let duration = humantime::parse_duration(&config.delay).context("Invalid delay format")?;

		if config.api_key == "YOUR_UNSPLASH_API_KEY" {
			time::sleep(time::Duration::from_secs(10)).await;
			continue;
		}

		let idle_seconds = UserIdle::get_time().map(|i| i.as_seconds()).unwrap_or(0);
		if idle_seconds < 600 && let Err(e) = update(&client, &config, &image_path).await {
			eprintln!("Error occurred during update: {:#}", e);
		}
		time::sleep(duration).await;
	}
}

async fn update(client: &reqwest::Client, config: &Config, image_path: &PathBuf) -> Result<()> {
	let (max_w, max_h) = DisplayInfo::all()
		.unwrap_or_default()
		.iter()
		.map(|d| (d.width, d.height))
		.max()
		.unwrap_or((1920, 1080));

	let photo: UnsplashPhoto = loop {
		let query = config
			.sources
			.choose(&mut rand::rng())
			.context("Sources list is empty")?;
		let params = [("query", query.as_str()), ("orientation", "landscape")];

		let response = client
			.get("https://api.unsplash.com/photos/random")
			.query(&params)
			.header("Authorization", format!("Client-ID {}", config.api_key))
			.send()
			.await?;

		if response.status().is_success() {
			let candidate: UnsplashPhoto = response.json().await?;
			if candidate.width >= max_w && candidate.height >= max_h {
				break candidate;
			}
		}
		time::sleep(time::Duration::from_secs(5)).await;
	};

	let bytes = client.get(&photo.urls.raw).send().await?.bytes().await?;
	fs::write(image_path, &bytes).await?;

	wallpaper::set_from_path(image_path.to_str().unwrap()).map_err(|e| anyhow!(e.to_string()))?;
	wallpaper::set_mode(wallpaper::Mode::Crop).map_err(|e| anyhow!(e.to_string()))?;

	Ok(())
}
