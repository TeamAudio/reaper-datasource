#!/usr/bin/env python3
"""Publish reaper-datasource workflow artifacts to the TeamAudio ReaPack repo.

Forked from reaspeech-lib's scripts/publish_reapack.py (single package, no
backend variants); keep improvements in sync where it makes sense.
"""

from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path


PROJECT_DIR = Path(__file__).resolve().parent.parent
GITHUB_REPOSITORY = "TeamAudio/reaper-datasource"

ARTIFACTS = {
    "reaper-datasource-windows-x86_64": "reaper_datasource.dll",
    "reaper-datasource-linux-x86_64": "reaper_datasource.so",
    "reaper-datasource-macos-arm64": "reaper_datasource.dylib",
}

PACKAGE_FILENAME = "reaper-datasource.ext"
PACKAGE_DESCRIPTION = "reaper-datasource (Excel/CSV parsing API for ReaScripts)"

DOC_FILE = PROJECT_DIR / "docs" / "reaper_datasource_doc.html"

# (platform or None, install target, release asset name)
PACKAGE_PROVIDES = [
    ("win64", "reaper_datasource.dll", "reaper_datasource.dll"),
    ("linux64", "reaper_datasource.so", "reaper_datasource.so"),
    ("darwin-arm64", "reaper_datasource.dylib", "reaper_datasource.dylib"),
    (None, "Data/reaper_datasource_doc.html", "reaper_datasource_doc.html"),
]

PACKAGE_LINKS = [
    "GitHub https://github.com/TeamAudio/reaper-datasource",
    "Tech Audio https://techaud.io",
]

PACKAGE_ABOUT = """\
# reaper-datasource

Native tabular-data parsing for REAPER ReaScripts — spreadsheets and CSV,
no backend required. Functions appear in the `reaper` namespace; all calls
are synchronous and return JSON strings.

- Parse whole workbooks (.xlsx/.xlsm/.xlsb/.xls/.ods/.csv/.tsv) in one call
- Or open a handle and query sheets, row ranges, and cells incrementally
- Hidden-row detection, delimiter sniffing, unbounded result sizes

After installing, run `reaper.DataSource_OpenDocs()` from any script to
open the bundled API reference in your browser."""


class PublishError(RuntimeError):
    pass


def run(*command: str | Path, cwd: Path | None = None, capture: bool = False) -> str:
    result = subprocess.run(
        [str(part) for part in command],
        cwd=cwd,
        check=True,
        text=True,
        stdout=subprocess.PIPE if capture else None,
    )
    return result.stdout.strip() if capture else ""


def package_version() -> str:
    in_package = False
    for line in (PROJECT_DIR / "Cargo.toml").read_text(encoding="utf-8").splitlines():
        stripped = line.strip()
        if stripped.startswith("["):
            in_package = stripped == "[package]"
        elif in_package and stripped.startswith("version"):
            _, value = stripped.split("=", 1)
            return value.strip().strip('"')
    raise PublishError("could not read the package version from Cargo.toml")


def validate_reascripts(path: Path, release: bool) -> None:
    if not (path / ".git").is_dir():
        raise PublishError(f"not a git repository: {path}")
    # Untracked files are tolerated; tracked changes mean a half-done state.
    dirty = run(
        "git", "status", "--porcelain", "--untracked-files=no",
        cwd=path, capture=True,
    )
    if release and dirty:
        raise PublishError("reascripts contains changes; --release requires a clean checkout")


def download_artifacts(run_id: str, destination: Path) -> None:
    if shutil.which("gh") is None:
        raise PublishError("GitHub CLI (gh) is required to download workflow artifacts")

    raw = run(
        "gh", "run", "view", run_id,
        "--repo", GITHUB_REPOSITORY,
        "--json", "conclusion,headSha,workflowName",
        capture=True,
    )
    details = json.loads(raw)
    if details.get("conclusion") != "success":
        raise PublishError(f"workflow run {run_id} did not succeed")
    if details.get("workflowName") != "Build":
        raise PublishError(f"run {run_id} is not from the Build workflow")

    source_sha = run("git", "rev-parse", "HEAD", cwd=PROJECT_DIR, capture=True)
    if details.get("headSha") != source_sha:
        raise PublishError(
            f"run commit {details.get('headSha')} does not match source HEAD {source_sha}"
        )

    print(f"Downloading artifacts from Build run {run_id}...")
    run(
        "gh", "run", "download", run_id,
        "--repo", GITHUB_REPOSITORY,
        "--dir", destination,
    )


def validate_artifacts(path: Path) -> None:
    for artifact, filename in ARTIFACTS.items():
        expected = path / artifact / filename
        if not expected.is_file():
            raise PublishError(f"missing artifact file: {expected}")


def stage_release_assets(artifacts: Path, destination: Path) -> list[Path]:
    destination.mkdir(parents=True, exist_ok=True)
    assets = []
    for artifact, filename in ARTIFACTS.items():
        output = destination / filename
        shutil.copy2(artifacts / artifact / filename, output)
        assets.append(output)
    if not DOC_FILE.is_file():
        raise PublishError(f"missing documentation file: {DOC_FILE}")
    doc_asset = destination / "reaper_datasource_doc.html"
    shutil.copy2(DOC_FILE, doc_asset)
    assets.append(doc_asset)
    return assets


def release_asset_url(tag: str, filename: str) -> str:
    return f"https://github.com/{GITHUB_REPOSITORY}/releases/download/{tag}/{filename}"


def stage_package(reascripts: Path, version: str, changelog: str, tag: str) -> None:
    extensions = reascripts / "Extensions"
    extensions.mkdir(parents=True, exist_ok=True)
    lines = [
        f"@description {PACKAGE_DESCRIPTION}",
        f"@version {version}",
        "@author Tech Audio",
        *(f"@link {link}" for link in PACKAGE_LINKS),
        "@about",
        *(f"  {line}" if line else "" for line in PACKAGE_ABOUT.splitlines()),
        "@changelog",
        f"  {changelog}",
        "@provides",
        *(
            f"  [{platform}] {target} {release_asset_url(tag, asset)}"
            if platform
            else f"  {target} {release_asset_url(tag, asset)}"
            for platform, target, asset in PACKAGE_PROVIDES
        ),
        "",
    ]
    (extensions / PACKAGE_FILENAME).write_text(
        "\n".join(lines), encoding="utf-8", newline="\n"
    )


def create_github_release(tag: str, version: str, changelog: str, assets: list[Path]) -> None:
    if shutil.which("gh") is None:
        raise PublishError("GitHub CLI (gh) is required for --release")

    existing = subprocess.run(
        ["gh", "release", "view", tag, "--repo", GITHUB_REPOSITORY],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    if existing.returncode == 0:
        raise PublishError(
            f"GitHub release {tag} already exists; refusing to replace its assets"
        )

    source_sha = run("git", "rev-parse", "HEAD", cwd=PROJECT_DIR, capture=True)
    run(
        "gh", "release", "create", tag,
        *(str(asset) for asset in assets),
        "--repo", GITHUB_REPOSITORY,
        "--target", source_sha,
        "--title", f"reaper-datasource {version}",
        "--notes", changelog,
    )


def publish_reapack(reascripts: Path, version: str) -> None:
    if shutil.which("reapack-index") is None:
        raise PublishError("reapack-index is required for --release")
    run("git", "add", "--all", "Extensions", cwd=reascripts)
    run("git", "commit", "-m", f"Update reaper-datasource to {version}", cwd=reascripts)
    run("reapack-index", "--commit", reascripts)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Stage GitHub Actions artifacts in the TeamAudio ReaPack repository."
    )
    parser.add_argument("--run-id", required=True, help="Build workflow run ID")
    parser.add_argument(
        "--reascripts",
        type=Path,
        default=PROJECT_DIR.parent / "reascripts",
        help="ReaPack repository (default: ../reascripts)",
    )
    parser.add_argument(
        "--artifacts-dir",
        type=Path,
        help="use already-downloaded artifacts instead of gh",
    )
    parser.add_argument("--changelog", help="ReaPack changelog text")
    parser.add_argument(
        "--tag",
        help="GitHub release tag (default: vVERSION)",
    )
    parser.add_argument(
        "--release",
        action="store_true",
        help="commit staged files and run reapack-index",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    reascripts = args.reascripts.expanduser().resolve()
    version = package_version()
    changelog = args.changelog or f"reaper-datasource {version}"
    tag = args.tag or f"v{version}"
    validate_reascripts(reascripts, args.release)

    with tempfile.TemporaryDirectory(prefix="reaper-datasource-reapack-") as temp:
        temp_dir = Path(temp)
        if args.artifacts_dir:
            artifacts = args.artifacts_dir.expanduser().resolve()
        else:
            artifacts = temp_dir / "workflow-artifacts"
            download_artifacts(args.run_id, artifacts)
        validate_artifacts(artifacts)
        assets = stage_release_assets(artifacts, temp_dir / "release-assets")
        stage_package(reascripts, version, changelog, tag)

        print(f"Staged reaper-datasource {version} descriptor in {reascripts / 'Extensions'}")
        if args.release:
            create_github_release(tag, version, changelog, assets)
            publish_reapack(reascripts, version)
            print(
                f"Published {tag} assets and indexed ReaPack. "
                "Review the reascripts commits, then push them manually."
            )
        else:
            print(
                f"Release assets would be uploaded to {GITHUB_REPOSITORY} as {tag}. "
                "Review the descriptor, then clean/restore reascripts and rerun with --release."
            )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (PublishError, subprocess.CalledProcessError, json.JSONDecodeError) as error:
        print(f"error: {error}", file=sys.stderr)
        raise SystemExit(1)
