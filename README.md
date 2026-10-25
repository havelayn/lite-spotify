# lite-spotify

A small, low-memory Spotify player for Windows. It uses your own **Spotify Premium** account.
Built in Rust with [egui](https://github.com/emilk/egui) for the window and
[librespot](https://github.com/librespot-org/librespot) for audio.

What it can do:

- **Library:** Liked Songs and your playlists, in a simple song table. Type in
  **Filter this list** to find a song, and click a column title (Title, Artist, Album,
  Time) to sort by it. Play then follows the order you see.
- **Heart button:** save the song that is playing to Liked Songs (or click the heart in
  any song row).
- **Search:** songs, albums, playlists and podcasts, with recent searches and quick ideas
  (Chill, Workout, Focus, …) for when you don't know what to look for.
- **Your own playlists:** make new ones, add songs (right-click a song → Add to playlist,
  or "+ Add to playlist" under the song that is playing), remove songs and rename them.
- **Similar songs:** after a single song, or when a list ends, lite-spotify keeps playing
  shuffled songs in the same style (from Spotify's own radio). Turn it off in Settings.
- **Podcasts:** the shows you follow and their episodes.
- **Lyrics:** click **Lyrics** under the volume. The line being sung is highlighted (click a
  line to jump there). **Full screen** turns it into karaoke on the song's own colour;
  press Esc to leave. Not every song has lyrics.
- **Mini player:** the small-window button at the top right makes a little player that
  stays on top of other windows. The arrows button brings the full window back.
- **Tray icon (Windows):** lite-spotify has an icon next to the clock. Right-click it for
  play/pause, next, previous, mini player and Quit. In Settings you can make closing the
  window keep the music playing there.
- **Spotify Connect (optional):** turn it on in Settings → Playback. lite-spotify then
  appears as a device called "lite-spotify" in the Spotify app on your phone, so you can
  pick music there and hear it here. The buttons in lite-spotify control it too. Off by
  default because it uses a second connection (a bit more memory).
- **Discord status (optional):** show "Listening to …" with the song in your Discord
  status. See *Discord status* below.
- **Stats tab:** how long you listened, your top songs and artists, and at what times you
  listen, for the last week, month, year or all time. Kept only on your computer.
- **Player panel** on the right: cover, seek bar, play/pause, next/previous, shuffle,
  repeat, volume, and an **Up next** list. Right-click a song to add it to the queue.
- **Fun tab** for when you don't know what to listen to:
  - **Daily Pick:** one song from your list, a new one every day.
  - **Guess the Song:** hear 5 seconds of a song from your list and pick the right title
    out of four. Need more? +5 seconds, for fewer points. Your own song comes back
    (paused) when you leave the game.
  - **Song Roulette:** spin the wheel.
  - **Claw Machine:** grab a mystery song (it slips sometimes).
  - **Slot Machine:** a line-up of three songs.
  - **Mystery Cards:** turn over one of six cards.
  - **This or That:** pick between two songs until one is left.
  - **Time Machine:** songs from a year or decade.
  - **Rediscover:** songs you saved long ago.
  - **Surprise Mix:** random songs from all over your playlists.
- **Media keys:** the play/pause, next and previous keys on your keyboard work, and
  Windows shows the song (with its cover) in the media popup next to the volume.
- **Continues where you left off:** when you start the app, the last song and the queue
  come back, paused at the same spot.
- **Sleep timer:** under the volume. The music fades out and pauses after 15–90 minutes,
  or after the current song.
- **Keyboard shortcuts:** Space play/pause, Ctrl+→/← next/previous, →/← skip 10 seconds,
  +/− volume, M mute, L like, S shuffle, R repeat, Ctrl+F search, Ctrl+1…5 tabs.
  The full list is in Settings.
- **Update check:** when a new version is out, a banner with a **Download** button
  appears at the top.
- **Settings:** dark or light look, accent colour (or **Album**: the colour of the cover
  that is playing), turn the Fun tab on or off, account and log file.

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

1. Open <https://github.com/havelayn/lite-spotify/releases>. (On the repository page,
   you can also click **Releases** on the right-hand side.)
2. The newest version is at the top. Under **Assets**, click
   `lite-spotify-vX.Y.Z.exe` to download it. You don't need to unzip anything.
3. Put the file anywhere you like, e.g. your Desktop, and double-click it.
   Windows may show "Windows protected your PC" because the app is not signed.
   Click **More info** and then **Run anyway**.

**Older versions** stay on the Releases page forever. Scroll down to find them.
All versions share the same settings and login, so you can switch back and forth.

**Test builds:** every change also gets built in the **Actions** tab. Open a run with
a green check mark and download **lite-spotify-windows** under **Artifacts**
(you need to be logged in to GitHub). These test builds are deleted after 90 days.

## 3. First start

1. Paste your **Client ID** into the box and click **Save and sign in**.
2. Your browser opens. Log in to Spotify and click **Agree**. Then go back to lite-spotify.
3. A second browser tab opens and asks you to allow playback. This is Spotify's own
   login for the audio player (Spotify only allows its own apps to stream audio).
   Click **Agree** again.
4. Done. Double-click a song (or click its number) to play it. Press **Space** to pause.
   Right-click a song to add it to the queue.

Next time you start the app you are signed in automatically.

## Good to know

- **Where are my settings?** In `%APPDATA%\lite-spotify` (paste that into the Explorer
  address bar). `config.json` holds your Client ID, login token and volume; `session.json` the song and
  queue to continue with; `history.jsonl` your listening history for the Stats tab;
  the `player` folder holds the player's saved login. `lite-spotify.log` is the log file.
  Please attach it when you report a bug.
- **Sign out / use a different Client ID:** open the **Settings** tab.
- **"One more permission":** making playlists and saving songs to Liked Songs need
  permissions that older versions didn't ask for. The app asks you to sign in once more
  the first time you use them.
- **Updating:** click **Download** in the banner (or get the newest `.exe` from Releases),
  close the old app and start the new file. Your settings and login stay. You can delete
  the old `.exe`.
- **Playlist changes refused:** Spotify limits what personal developer apps may change. If
  Spotify refuses, the app says so; you can still make the change in the Spotify app.
- **Turn off the Fun tab:** Settings → Fun tab. The games use almost no memory, and none
  while turned off.
- **Some playlists do not open:** Spotify limits what developer apps may read. If a playlist
  you follow but did not create can't be read through the Web API, lite-spotify tries to
  load it through the player connection instead.
- **Some podcasts won't play:** video podcasts and podcasts Spotify doesn't host itself
  may refuse to play here; lite-spotify then skips to the next one.
- **Port in use:** sign-in needs ports 8888 and 8898 on your own computer for a moment.
  If another program uses them, close it and try again.
- Never share your `config.json` or the `player` folder: they contain your login.

## Discord status

Discord only shows a status for an "application" registered with Discord, which you make
yourself. It's free and takes two minutes:

1. Open <https://discord.com/developers/applications> and log in with your Discord account.
2. Click **New Application**, name it `lite-spotify` (Discord shows this name in your
   status), accept the terms and click **Create**.
3. On the application's page, copy the **Application ID** (a long number).
4. In lite-spotify, open **Settings → Discord status**, paste the ID and tick
   **Show the song I'm listening to in Discord**.

The Discord app has to run on the same computer. The status only shows while music plays.

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
