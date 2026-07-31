"""`python -m procora` 的轻量包构建入口。"""

import argparse

from .package import build


def main() -> None:
    """解析常用构建参数并输出结果。"""
    parser = argparse.ArgumentParser(prog="python -m procora")
    parser.add_argument("source", nargs="?", default=".")
    parser.add_argument("-o", "--output")
    parser.add_argument("--platform", default="all")
    parser.add_argument("--prepare", action="append", default=[])
    parser.add_argument("--force", action="store_true")
    args = parser.parse_args()
    result = build(
        args.source,
        output=args.output,
        platform=args.platform,
        prepare=args.prepare,
        force=args.force,
    )
    print(result.path)


if __name__ == "__main__":
    main()
