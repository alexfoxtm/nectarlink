# Android updates — Nectar R2 manifest

The Nectar-based Android app checks one stable manifest. It does not call
the GitHub releases API. Cloud has its own feed and is not read or written
here.

```
https://pub-a57457f4da43415ea6335550ee9e2983.r2.dev/waypair/nectar/android/stable/update.json
```

Origin `pub-a57457f4da43415ea6335550ee9e2983.r2.dev`, path prefix
`/waypair/nectar/android/`. This is the Nectar route, parallel to
`waypair/nectar/macos/stable/appcast.xml`, and it is not
`waypair/android/stable/update.json` or `waypair/cloud/android/stable/update.json`.

## Manifest

The body keeps the GitHub release shape. A release is installable only when
all of the following hold:

| Field | Rule |
| --- | --- |
| `tag_name` | Present, 1–64 characters, no whitespace. An optional leading `v` is stripped. The remainder must be newer than the running version (`1.2.3`, optional `-pre`). |
| `html_url` | Present, `https://`, no whitespace, at most 2048 characters. Notes only; it is not downloaded. |
| `draft` | Present JSON boolean, `false`. |
| `prerelease` | Present JSON boolean, `false`. |
| `assets` | JSON array, 1–32 objects. Exactly one `SHA256SUMS.txt`. Exactly one APK whose `name` is a single segment matching `*-android.apk` (letters, digits, `-`, `.`, `_`, no `..`). Upstream's release job publishes `Nectarlink-<version>-android.apk`. Other assets are ignored. |

Each of those two assets has `browser_download_url` on the origin and prefix
above, and the URL's last path segment equals `name`. Example:

```json
{
  "tag_name": "v9.0.0",
  "html_url": "https://github.com/thtbee/nectarlink/releases/tag/v9.0.0",
  "draft": false,
  "prerelease": false,
  "assets": [
    {
      "name": "Nectarlink-9.0.0-android.apk",
      "browser_download_url": "https://pub-a57457f4da43415ea6335550ee9e2983.r2.dev/waypair/nectar/android/stable/Nectarlink-9.0.0-android.apk"
    },
    {
      "name": "SHA256SUMS.txt",
      "browser_download_url": "https://pub-a57457f4da43415ea6335550ee9e2983.r2.dev/waypair/nectar/android/stable/SHA256SUMS.txt"
    }
  ]
}
```

`SHA256SUMS.txt` is the same line format as before: 64 hex digits, whitespace,
then the file name (an optional `*` before the name is ignored). The download
is kept only when its SHA-256 matches the line for the APK name.

## What the client refuses

Manifest, sums, and APK requests are HTTPS GETs. `HttpURLConnection` does not
follow redirects itself. At most three redirects are followed, and only when
the next URL still passes the same rule: scheme `https`, host exactly the
origin above, port omitted or 443, no userinfo, query, fragment, or
percent-encoding, path under `/waypair/nectar/android/` with no `..`.

| Response | Result |
| --- | --- |
| `404` on the manifest | No release. The app stays up to date. The body is not parsed. |
| Any other HTTP status, or a transport failure | The check fails (`Reason.Offline`). It is not treated as "no release". |
| `200` that is not an installable release (draft, prerelease, not newer, missing or duplicate APK or sums, URL off this origin or path) | No update is offered. |
| Malformed JSON | The check fails. |

Bodies are capped: manifest 1 MiB, `SHA256SUMS.txt` 64 KiB, APK 300 MiB.
There is no production URL override and no injected update.

## Install

The resumable download is unchanged: a partial file continues with
`Range`, a dropped network waits and retries, and a finished APK is renamed
only after the checksum matches. Android's `PackageInstaller` session is a
full install for this package. The OS installs it over the installed app only
when the signatures match; a different signer fails and the app reports
`Reason.Install`. Debug builds do not update themselves.

This client only reads the public manifest and its assets. It does not write
to R2, does not use cloud or signing credentials, and does not add
dependencies or Gradle configuration.
