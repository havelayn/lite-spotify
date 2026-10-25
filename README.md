# lite-spotify

A small, low-memory Spotify player for Windows. It uses your own **Spotify Premium** account.
Built in Rust with [egui](https://github.com/emilk/egui) for the window and
[librespot](https://github.com/librespot-org/librespot) for audio.

It can: sign in, show your playlists and Liked Songs, play songs, play/pause, next/previous,
seek, volume, show what's playing (with a small cover), and search songs, albums and playlists.

Streaming only: nothing is downloaded or saved, and Premium is required for playback.

## 1. Create your own Spotify developer app (one time, ~3 minutes)

lite-spotify talks to Spotify through a free "developer app" that belongs to you.

1. Go to <https://developer.spotify.com/dashboard> and log in with your Spotify account.
   Accept the developer terms if asked.
2. Click **Create app**.
3. Fill in:
   - **App name**: anything, e.g. `lite-spotify`
   - **App description**: anything, e.g. `my player`
   - **Redirect URIs**: type exactly `http://127.0.0.1:8888/callback` and click **Add**.
     (Use `127.0.0.1`, not `localhost`. Spotify no longer accepts `localhost`.)
   - Under "Which API/SDKs are you planning to use?", tick **Web API**.
4. Tick the terms box and click **Save**.
5. On your app's page, open **Settings**. Copy the **Client ID** (32 letters and numbers).
   You do **not** need the Client secret.

## 2. Download and run lite-spotify

1. Open this repository on GitHub and click the **Actions** tab.
2. Click the newest run of **Build** that has a green check mark.
3. Scroll down to **Artifacts** and click **lite-spotify-windows**. A `.zip` file downloads.
   (You need to be logged in to GitHub to download artifacts.)
4. Unzip it. Inside is `lite-spotify.exe`. Put it anywhere you like, e.g. your Desktop.
5. Double-click `lite-spotify.exe`.
   Windows may show "Windows protected your PC" because the app is not signed.
   Click **More info** and then **Run anyway**.

## 3. First start

1. Paste your **Client ID** into the box and click **Save and sign in**.
2. Your browser opens. Log in to Spotify and click **Agree**. Then go back to lite-spotify.
3. A second browser tab opens and asks you to allow playback. This is Spotify's own
   login for the audio player (Spotify only allows its own apps to stream audio).
   Click **Agree** again.
4. Done. Double-click a song (or click its number) to play it. Press **Space** to pause.

Next time you start the app you are signed in automatically.

## Good to know

- **Where are my settings?** In `%APPDATA%\lite-spotify` (paste that into the Explorer
  address bar). `config.json` holds your Client ID, login token and volume;
  the `player` folder holds the player's saved login. `lite-spotify.log` is the log file.
  Please attach it when you report a bug.
- **Sign out:** click the ⚙ button at the top right and choose **Sign out**.
- **Use a different Client ID:** sign out, then click **Use a different Client ID**.
- **Some playlists do not open:** Spotify limits what developer apps may read. If a playlist
  you follow but did not create can't be read through the Web API, lite-spotify tries to
  load it through the player connection instead.
- **Port in use:** sign-in needs ports 8888 and 8898 on your own computer for a moment.
  If another program uses them, close it and try again.
- Never share your `config.json` or the `player` folder: they contain your login.

## Building it yourself (optional)

Install Rust from <https://rustup.rs>, then:

```
cargo build --release
```

The program ends up in `target\release\lite-spotify.exe`. On Linux you also need the
ALSA headers (`sudo apt install libasound2-dev`).

## Credits

The app uses the [Inter](https://rsms.me/inter/) font by Rasmus Andersson, under the
SIL Open Font License (see `assets/fonts/Inter-LICENSE.txt`).
