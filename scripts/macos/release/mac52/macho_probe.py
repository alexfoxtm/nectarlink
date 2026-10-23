"""Root-only checked Mach-O inspection; importing this module starts no tool."""
import re
import subprocess
import sys


def checked_output(tool, *arguments):
    if tool not in {'otool', 'lipo', 'vtool'}:
        raise ValueError('unsupported Mach-O inspection tool')
    result = subprocess.run([tool, *arguments], capture_output=True, text=True, timeout=300)
    if result.returncode != 0 or result.stderr.strip() or not result.stdout.strip():
        raise RuntimeError(f'{tool} inspection failed: exit={result.returncode}; stderr={result.stderr[:2000]!r}; empty={not result.stdout.strip()}')
    if re.search(r'(?im)^.*(?:\b(?:error|fatal|warning)\s*:|\bnot an object file\b|\bmalformed\b|\btruncated\b)', result.stdout):
        raise RuntimeError(f'{tool} returned diagnostic output: {result.stdout[:2000]!r}')
    return result.stdout


if __name__ == '__main__':
    try:
        sys.stdout.write(checked_output(sys.argv[1], *sys.argv[2:]))
    except Exception as error:
        print(str(error), file=sys.stderr)
        raise SystemExit(1)
