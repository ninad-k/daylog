import hashlib
from pathlib import Path
import tempfile
import unittest

from release_config import make_config
from winget import generate


class ReleaseConfigTests(unittest.TestCase):
    def test_unsigned_build_has_no_update_artifacts(self):
        self.assertFalse(make_config({}, "0.1.0")["bundle"]["createUpdaterArtifacts"])

    def test_signing_requires_public_key_and_correct_version(self):
        with self.assertRaises(ValueError):
            make_config({"TAURI_SIGNING_PRIVATE_KEY": "secret"}, "0.1.0")
        with self.assertRaises(ValueError):
            make_config({"GITHUB_REF": "refs/tags/v0.2.0"}, "0.1.0")

    def test_updater_config_contains_no_private_key(self):
        config = make_config(
            {
                "TAURI_SIGNING_PRIVATE_KEY": "secret",
                "TAURI_UPDATER_PUBLIC_KEY": "public",
                "GITHUB_REPOSITORY": "owner/daylog",
            },
            "0.1.0",
        )
        self.assertTrue(config["bundle"]["createUpdaterArtifacts"])
        self.assertEqual(config["plugins"]["updater"]["pubkey"], "public")
        self.assertNotIn("secret", str(config))
        self.assertIn("owner/daylog", config["plugins"]["updater"]["endpoints"][0])

    def test_public_key_without_private_key_can_check_signed_releases(self):
        config = make_config({"TAURI_UPDATER_PUBLIC_KEY": " public \n"}, "0.1.0")
        self.assertFalse(config["bundle"]["createUpdaterArtifacts"])
        self.assertEqual(config["plugins"]["updater"]["pubkey"], "public")

    def test_update_endpoint_must_be_complete_https_url(self):
        for endpoint in ["http://example.com/latest.json", "https://", "file:///latest.json"]:
            with self.subTest(endpoint=endpoint), self.assertRaises(ValueError):
                make_config(
                    {"TAURI_UPDATER_PUBLIC_KEY": "public", "TAURI_UPDATER_ENDPOINT": endpoint},
                    "0.1.0",
                )

    def test_valid_tag_and_windows_certificate_config(self):
        config = make_config(
            {"GITHUB_REF": "refs/tags/v0.1.0", "WINDOWS_CERTIFICATE_THUMBPRINT": "A" * 40},
            "0.1.0",
        )
        self.assertEqual(config["bundle"]["windows"]["digestAlgorithm"], "sha256")
        with self.assertRaises(ValueError):
            make_config({"WINDOWS_CERTIFICATE_THUMBPRINT": "invalid"}, "0.1.0")


class WingetTests(unittest.TestCase):
    def test_manifest_hash_comes_from_actual_installer(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            installer = root / "daylog_0.1.0_x64_en-US.msi"
            installer.write_bytes(b"test installer")
            output = generate(installer, "0.1.0", "owner/daylog", root / "manifests")
            self.assertEqual(len(list(output.glob("*.yaml"))), 3)
            body = (output / "NinadK.daylog.installer.yaml").read_text(encoding="utf-8")
            self.assertIn(hashlib.sha256(installer.read_bytes()).hexdigest().upper(), body)
            self.assertIn("/releases/download/v0.1.0/daylog_0.1.0_x64_en-US.msi", body)

    def test_asset_filename_is_url_encoded(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            installer = root / "daylog test #1%.msi"
            installer.write_bytes(b"test installer")
            output = generate(installer, "0.1.0+test", "owner/daylog", root / "manifests")
            body = (output / "NinadK.daylog.installer.yaml").read_text(encoding="utf-8")
            self.assertIn("/v0.1.0%2Btest/daylog%20test%20%231%25.msi", body)

    def test_missing_msi_and_path_like_version_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            installer = root / "daylog.msi"
            with self.assertRaises(ValueError):
                generate(installer, "0.1.0", "owner/daylog", root / "manifests")
            installer.write_bytes(b"test installer")
            with self.assertRaises(ValueError):
                generate(installer, "../other", "owner/daylog", root / "manifests")


if __name__ == "__main__":
    unittest.main()
