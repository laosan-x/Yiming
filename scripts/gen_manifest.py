#!/usr/bin/env python3
"""全量生成 update-manifest.json（由 release.yml 在发布 GitHub Release 后调用）。

设计契约
--------
main 分支上的 update-manifest.json **永远是一个已发布版本的完整清单**，
不存在 ``size: 0`` / ``sha256: "PENDING"`` 的骨架窗口。

旧流程的缺陷：发版前先 push 骨架清单，CI 构建 ~15 分钟 + 人工回填期间，
CDN 上一直是带 PENDING 摘要的新版本清单。旧版客户端此时点「一键更新」，
会正常下载安装包，但 SHA-256 与字面量 "PENDING" 必然不匹配 → 换源重下
→ 四个源全部失败。安全性没破（不会装错包），但用户白下 ~40MB×4 然后报错。

新流程：发版时**不碰** update-manifest.json（它仍是上一个版本的完整清单，
窗口期内旧版客户端只会看到「已是最新版本」）；CI 构建并发布安装包后，
用本脚本从真实产物计算 size/sha256，一次性生成完整清单并 push 到 main，
再由 purge-cdn.yml 刷新 CDN。

notes 的唯一数据源是 release-notes.txt（发版时与 Cargo.toml 一同更新），
避免更新清单与设置页更新日志两处手写漂移。

用法::

    python scripts/gen_manifest.py \
        --version 0.4.5 \
        --notes-file release-notes.txt \
        --asset dist/cocole-iced-v0.4.5-setup.exe \
        [--out update-manifest.json] [--repo laosan-x/Yiming]
"""

import argparse
import datetime
import hashlib
import json
import pathlib
import sys

DEFAULT_REPO = "laosan-x/Yiming"

# 境内可达镜像（与 updater 的多源下载顺序一致）
MIRRORS = (
    "https://gh-proxy.com/https://github.com",
    "https://ghfast.top/https://github.com",
    "https://ghproxy.net/https://github.com",
)


def sha256_file(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def build_manifest(version: str, notes: str, asset: pathlib.Path, repo: str) -> dict:
    v = version.lstrip("v")
    if not v or not all(part.isdigit() for part in v.split(".")):
        raise ValueError(f"非法版本号: {version!r}（应为 0.4.5 或 v0.4.5）")
    if not asset.is_file():
        raise FileNotFoundError(f"安装包不存在: {asset}")
    if not notes.strip():
        raise ValueError("release-notes.txt 为空")

    release_url = (
        f"https://github.com/{repo}/releases/download/"
        f"v{v}/cocole-iced-v{v}-setup.exe"
    )
    return {
        "version": v,
        "notes": notes.strip(),
        # CI runner 与本地都统一用 UTC 日期，避免跨时区发布时日期回退
        "pub_date": datetime.datetime.now(datetime.timezone.utc).date().isoformat(),
        "mandatory": False,
        "platforms": {
            "windows-x86_64": {
                "url": release_url,
                "mirror_urls": [
                    f"{m}/{repo}/releases/download/v{v}/cocole-iced-v{v}-setup.exe"
                    for m in MIRRORS
                ],
                "size": asset.stat().st_size,
                "sha256": sha256_file(asset),
            }
        },
    }


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--version", required=True, help="版本号，来自 tag（v0.4.5 / 0.4.5）")
    ap.add_argument("--notes-file", required=True, help="纯文本更新说明文件")
    ap.add_argument("--asset", required=True, help="已构建的 NSIS 安装包路径")
    ap.add_argument("--out", default="update-manifest.json", help="输出 JSON 路径")
    ap.add_argument("--repo", default=DEFAULT_REPO, help="GitHub owner/repo")
    args = ap.parse_args()

    notes = pathlib.Path(args.notes_file).read_text(encoding="utf-8")
    manifest = build_manifest(args.version, notes, pathlib.Path(args.asset), args.repo)
    out = pathlib.Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    info = manifest["platforms"]["windows-x86_64"]
    print(
        f"generated {out}: v{manifest['version']} "
        f"size={info['size']} sha256={info['sha256']}"
    )
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (ValueError, FileNotFoundError, OSError) as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        sys.exit(1)
