"""Create a Tauri release override without storing signing secrets in source."""

import argparse
import json
import os
from pathlib import Path
from urllib.parse import urlsplit


def make_config(env, version):
    ref = env.get("GITHUB_REF", "")
    if ref.startswith("refs/tags/") and ref.removeprefix("refs/tags/v") != version:
        raise ValueError("Release tag must match tauri.conf.json version")
    public_key = env.get("TAURI_UPDATER_PUBLIC_KEY", "").strip()
    signing_key = env.get("TAURI_SIGNING_PRIVATE_KEY", "").strip()
    if signing_key and not public_key:
        raise ValueError("Set the TAURI_UPDATER_PUBLIC_KEY repository variable before signing updates")
    result = {"bundle": {"createUpdaterArtifacts": bool(signing_key)}}
    if public_key:
        repository = env.get("GITHUB_REPOSITORY", "ninad-k/daylog")
        endpoint = env.get("TAURI_UPDATER_ENDPOINT", "").strip() or (
            f"https://github.com/{repository}/releases/latest/download/latest.json"
        )
        parsed = urlsplit(endpoint)
        if parsed.scheme != "https" or not parsed.hostname:
            raise ValueError("The updater endpoint must use HTTPS")
        result["plugins"] = {"updater": {"pubkey": public_key, "endpoints": [endpoint]}}
    thumbprint = env.get("WINDOWS_CERTIFICATE_THUMBPRINT", "").strip()
    if thumbprint:
        if len(thumbprint) != 40 or any(c not in "0123456789abcdefABCDEF" for c in thumbprint):
            raise ValueError("Invalid Windows certificate thumbprint")
        result["bundle"]["windows"] = {
            "certificateThumbprint": thumbprint,
            "digestAlgorithm": "sha256",
            "timestampUrl": "http://timestamp.digicert.com",
        }
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--config", type=Path, default=Path("src-tauri/tauri.conf.json"))
    args = parser.parse_args()
    version = json.loads(args.config.read_text(encoding="utf-8"))["version"]
    config = make_config(os.environ, version)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(config, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
