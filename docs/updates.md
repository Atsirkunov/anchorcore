# Updates

AnchorCore checks for a newer release **only when you ask it to** — there is no
background update ping. That is deliberate: the app is local-first, and an idle
connection to a server you never asked for would contradict that.

## In the app (System tab → Updates)

- **Check for updates** performs one request to `api.github.com` for the newest
  release tag (a plain fetch of public release info — nothing about you or your
  knowledge is sent) and shows `current` vs newest, a platform-matched download
  link and the release notes.
- A plain `GET /system/update` never performs that request: opening the app or
  the card only reads the cached result. (Pinned by the Rust test
  `plain_update_get_never_checks_network`.)
- When a check finds a newer release, a dismissible banner appears above the
  tabs; the dismissal is per version, so the next release shows up again.

## Opt-in startup check

Settings → *Updates* → **Check for new releases once at startup** (default
**off**) performs one check when the app starts, so the banner can appear on its
own.

- Setting `update_check_enabled` in `app_settings` (`1` = on, `0`/unset = off).
- Env `ANCHOR_UPDATE_CHECK_ENABLED=1` enables it for every launch; an explicit
  in-app `0` wins over the env value (DB overrides env, like every other setting).

## Updating the app

AnchorCore does not replace itself while running — Windows locks the running
`.exe`, and macOS expects a signed bundle. The card links the download; then:

1. quit the app,
2. unzip the newest `AnchorCore-{windows,macos,linux}.zip` over the install,
3. start it again.

Your knowledge stays in `~/.anchorcore`; migrations run on the next start.

Agents can do the whole flow: the
[`anchorcore-setup` skill](../.agents/skills/anchorcore-setup/SKILL.md) checks
the installed version against `releases/latest` and only replaces the binary
when it is strictly older (never downgrades a dev/source build). `anchorcore
--version` reports the on-disk build without starting it.

## API

`GET /system/update` → cached status. `GET /system/update?refresh=1` → performs
the check (one request a minute at most; GitHub allows 60 unauthenticated
requests/hour/IP).

```json
{
  "current": "1.0.21",
  "latest": "1.0.22",
  "update_available": true,
  "release_url": "https://github.com/Atsirkunov/anchorcore/releases/tag/v1.0.22",
  "published_at": "2026-10-12T07:00:00Z",
  "asset_url": "https://github.com/Atsirkunov/anchorcore/releases/latest/download/AnchorCore-windows.zip",
  "checked_at": "2026-10-12T09:31:00Z",
  "check_enabled": false,
  "error": null
}
```

Offline/error cases return `latest: null` with a plain-language `error` and HTTP
200 — the UI never breaks over an update check. `ANCHOR_UPDATE_URL` points the
check at a mirror or test stub (tests only; never a user-facing setting).
