# OTA Updates

## Server Layout (Local Network)

Serve a simple HTTP directory (no HTTPS required):

```
http://<host>/firmware/latest.txt
http://192.168.1.18:8000/firmware/c6-co123.bin
```

`latest.txt` should contain the firmware filename only, e.g.:

```
c6-co123.bin
```

Version comparison is done using the build number encoded in the filename
(`c6-co<build>.bin`). The current build is taken from `OTA_BUILD` (if set at
build time) or from the `+<build>` suffix in `CARGO_PKG_VERSION` (e.g.
`0.1.0+123`).

## Build/Upload

1) Build a release binary:
```
scripts/build-release.sh
```

2) Export/rename the app `.bin` for OTA:
```
scripts/export-ota.sh 0001 ./firmware
```

This writes `c6-co0001.bin` and `latest.txt` into `./firmware`.

Alternatively, use the combined helper to increment the build number,
build, and export in one step:
```
scripts/build-export.sh ./firmware
```

To export a debug build instead:
```
scripts/build-export.sh ./firmware debug
```

## Configuration

The firmware uses compile-time env vars:

- `WIFI_SSID` / `WIFI_PASS` (Wi-Fi credentials)
- `OTA_BASE_URL` (e.g. `http://192.168.1.18:8000/firmware`)

If not set, OTA checks are skipped because Wi-Fi init fails.

## Partition Table

OTA requires the custom partition table in `partitions.csv`
(factory + ota_0 + ota_1). Adjust sizes if your app grows beyond 1MB.
