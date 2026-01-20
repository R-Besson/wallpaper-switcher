# Wallpaper Switcher

Wallpaper switcher that fetches fresh new wallpapers from the web with a user-specified delay. Makes sure the wallpapers are higher resolution than your monitors.\
Currently uses Unsplash for source as they have the highest resolution wallpapers out there and filter user submitted wallpapers well.

## Building

Firstly install rust: https://rust-lang.org/tools/install/
```bash
git clone https://github.com/R-Besson/wallpaper-switcher.git
cd wallpaper-switcher
cargo build
```

## Usage
1. [Make an unsplash account](https://unsplash.com/) and verify your email.
1. [Make a new unsplash app.](https://unsplash.com/oauth/applications)
2. Grab the `Access Key` and paste it in the `api-key` field below.

### Example `config.json` file:
```json
{
	"api_key": "YOUR_UNSPLASH_API_KEY",
	"delay": "5m",
	"sources": [
		"nebula",
		"macro nature",
		"astrophotography",
		"long exposure"
	]
}
```

## Possible future features
- User Interface?
- Mosaic/Collage?
- Separate pictures for each monitor?
- More supported sources?