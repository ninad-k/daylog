"""Generate reviewable winget manifests from the actual release MSI."""

import argparse
import hashlib
import json
from pathlib import Path
from urllib.parse import quote


def generate(installer, version, repository, output):
    if installer.suffix.lower() != ".msi" or not installer.is_file():
        raise ValueError("An existing MSI installer is required")
    if not version.strip() or version in {".", ".."} or any(c in version for c in "/\\"):
        raise ValueError("The version must be a single directory name")
    digest = hashlib.sha256(installer.read_bytes()).hexdigest().upper()
    identifier = "NinadK.daylog"
    root = output / version
    root.mkdir(parents=True, exist_ok=True)
    # JSON quoted strings are valid YAML scalars and prevent punctuation in a
    # release version/URL from accidentally changing the manifest structure.
    def scalar(value):
        return json.dumps(value, ensure_ascii=False)

    common = f"PackageIdentifier: {identifier}\nPackageVersion: {scalar(version)}\n"
    url = f"https://github.com/{repository}/releases/download/v{quote(version, safe='')}/{quote(installer.name, safe='')}"
    manifests = {
        f"{identifier}.yaml": common
        + "DefaultLocale: en-US\nManifestType: version\nManifestVersion: 1.9.0\n",
        f"{identifier}.installer.yaml": common
        + "InstallerType: wix\nScope: machine\nInstallerLocale: en-US\nInstallModes:\n- interactive\n- silent\n- silentWithProgress\nUpgradeBehavior: install\nInstallers:\n- Architecture: x64\n"
        + f"  InstallerUrl: {scalar(url)}\n  InstallerSha256: {digest}\n"
        + "ManifestType: installer\nManifestVersion: 1.9.0\n",
        f"{identifier}.locale.en-US.yaml": common
        + "PackageLocale: en-US\nPublisher: Ninad K\nPackageName: daylog\nLicense: MIT\n"
        + f"LicenseUrl: https://github.com/{repository}/blob/main/LICENSE\n"
        + f"PackageUrl: https://github.com/{repository}\n"
        + "ShortDescription: Privacy-respecting personal activity logger.\n"
        + "Tags:\n- productivity\n- time-tracking\n- privacy\n"
        + "ManifestType: defaultLocale\nManifestVersion: 1.9.0\n",
    }
    for name, body in manifests.items():
        (root / name).write_text(body, encoding="utf-8")
    return root


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("installer", type=Path)
    parser.add_argument("--version", required=True)
    parser.add_argument("--repository", default="ninad-k/daylog")
    parser.add_argument("--output", type=Path, default=Path("dist/winget"))
    args = parser.parse_args()
    print(generate(args.installer, args.version, args.repository, args.output))


if __name__ == "__main__":
    main()
