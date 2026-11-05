#!/usr/bin/env python3
"""发版前版本一致性校验（纯标准库，无副作用，只读）。

新发版 SOP 下开发者手改的位置有 4 处，很容易漏掉某一处。本脚本把它们
集中校验，输出显式 pass/fail 计数，可在打 tag 前运行::

    python scripts/check-release.py

注意 `update-manifest.json` **不参与 fail 判定**：新流程里它在发版前是
上一个版本的完整清单、由 CI 发布后自动更新，要求它等于 Cargo.toml 反而是错的。
"""

import json
import pathlib
import re
import sys

# Windows CI runner（en-US）的 stdout 默认可能是 cp1252：脚本里的中文
# 检查项一 print 就抛 UnicodeEncodeError，门禁以 exit 1 失败，而所有检查
# 其实都通过了（v0.4.5 首次跑新发版流程时实测踩到）。输出统一强制 UTF-8，
# 控制台不支持时降级替换，绝不让输出编码影响退出码。
if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
if hasattr(sys.stderr, "reconfigure"):
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")

ROOT = pathlib.Path(__file__).resolve().parent.parent


def read(rel: str) -> str:
    return (ROOT / rel).read_text(encoding="utf-8")


def cargo_version() -> str:
    m = re.search(r'^version\s*=\s*"([^"]+)"', read("Cargo.toml"), re.M)
    if not m:
        raise SystemExit("FATAL: Cargo.toml 找不到 version 字段")
    return m.group(1)


def main() -> int:
    v = cargo_version()
    checks: list[tuple[str, bool, str]] = []

    def check(name: str, ok: bool, detail: str) -> None:
        checks.append((name, ok, detail))

    readme = read("README.md")
    check(
        "README 版本徽章",
        f"Version-{v}-" in readme,
        f"应含 Version-{v}-",
    )
    check(
        "README 安装包文件名",
        f"cocole-iced-v{v}-setup.exe" in readme,
        f"应含 cocole-iced-v{v}-setup.exe",
    )

    settings = read("src/ui/page_settings.rs")
    m = re.search(r'fn changelog_panel.*?\(\s*"([0-9]+\.[0-9]+\.[0-9]+)"', settings, re.S)
    first = m.group(1) if m else "<未找到>"
    check("设置页 changelog 首条", first == v, f"当前 {first}，应为 {v}")

    notes = read("release-notes.txt").splitlines()[0] if read("release-notes.txt").strip() else ""
    check("release-notes.txt 首行", f"v{v}" in notes, f"当前 {notes!r}，应含 v{v}")

    manifest = json.loads(read("update-manifest.json"))
    mv = manifest.get("version", "?")
    asset = manifest["platforms"]["windows-x86_64"]
    check(
        "清单格式（size/sha256 非骨架）",
        isinstance(asset.get("size"), int)
        and asset["size"] > 0
        and asset.get("sha256") not in (None, "", "PENDING"),
        f"version={mv} size={asset.get('size')}",
    )
    if mv != v:
        print(
            f"[INFO] update-manifest.json 当前为 {mv}（Cargo.toml 为 {v}）；"
            "发版前允许是上一版，CI 发布后会自动更新"
        )

    print(f"目标版本: {v}\n")
    passed = 0
    for name, ok, detail in checks:
        print(f"  [{'PASS' if ok else 'FAIL'}] {name}: {detail}")
        passed += int(ok)
    failed = len(checks) - passed
    print(f"\n{passed} passed / {failed} failed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
