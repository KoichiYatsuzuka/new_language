# analyze.py — sampler の折りたたみ形式（根;...;葉 回数）を集計する（scripts/prof_stacks.ps1 が呼ぶ）
#
#   python analyze.py <file.folded> [--top N] [--incl 名前,名前] [--under 名前] [--callers 名前]
#
# 見るのはインタープリタのスレッド（`interpreter_main` を含むスタック）だけ。出すもの:
#   1. from generated code (inclusive) … いちばん内側の生成コード（`<module>_tl.dll`）のフレームの次＝呼んだコールバックごとの
#      包括時間。`(generated code itself)` は葉が生成コード、`(outside generated code)` は生成コードを通らない標本
#   2. self time                       … 葉の関数ごとの時間
#   3. inclusive (name contains)       … `--incl` の文字列を名前に含む関数がスタックにある標本
#   4. callers of <名前>               … `--callers` の関数（いちばん外の出現）を直接呼んだ関数
# `--under 名前` を付けると、その関数（いちばん内側の出現）より下だけを数える。
#
# ⚠ 記号は exe の隣の PDB から引く。デバッグ情報なしでビルドした exe は内部の関数に記号が無く、近くの関数の名前に
#   化ける（copy_alloc_speed_plan.md 1-1）。`ntdll.dll` の中も公開された名前しか無いので、ヒープの内部の関数は
#   `RtlSizeHeap` などに寄る（「ヒープの確保・解放」とまとめて読む）。
import re
import sys
from collections import Counter

ESC = {'$LT$': '<', '$GT$': '>', '$u20$': ' ', '$RF$': '&', '$C$': ',', '$u7b$': '{', '$u7d$': '}', '$BP$': '*',
       '$u5b$': '[', '$u5d$': ']', '$u27$': "'", '$LP$': '(', '$RP$': ')', '$u3d$': '='}


def demangle(sym):
    """`module!name` を (module, 短くした名前) にする（Rust の古い形式の名前を読める形に戻す）。"""
    mod, _, name = sym.partition('!')
    name = re.sub(r'\.llvm\.\d+$', '', name)
    m = re.match(r'_?ZN(.*)E$', name)
    if m:
        s, parts, i = m.group(1), [], 0
        while i < len(s):
            j = i
            while j < len(s) and s[j].isdigit():
                j += 1
            if j == i:
                break
            n = int(s[i:j])
            parts.append(s[j:j + n])
            i = j + n
        if parts and re.fullmatch(r'h[0-9a-f]{16}', parts[-1]):
            parts.pop()
        name = '::'.join(parts)
        for k, v in ESC.items():
            name = name.replace(k, v)
        name = name.replace('..', '::')
        if name.startswith('_<') or name.startswith('_$'):
            name = name[1:]
    name = name.replace('arrow::interpreter::', 'i::')
    name = re.sub(r'core::ptr::drop_in_place<(.*)>$', r'drop<\1>', name)
    return mod, name


def load(path):
    stacks = []
    for line in open(path, encoding='utf-8', errors='replace'):
        line = line.rstrip('\n')
        if not line:
            continue
        body, _, cnt = line.rpartition(' ')
        if 'interpreter_main' not in body:
            continue
        stacks.append(([demangle(f) for f in body.split(';')], int(cnt)))
    return stacks


def is_generated(mod):
    """コンパイル済みモジュールの DLL か（実行時に `<module>_tl.dll` として読み込まれる）。"""
    return mod.lower().endswith('_tl.dll')


def is_heap(mod, name):
    """葉がヒープの確保・解放か: OS のヒープ（ntdll / kernel32）・Rust の確保の入口・mimalloc（`mi_*` / `_mi_*`）。"""
    return (mod.lower() in ('ntdll.dll', 'kernel32.dll', 'kernelbase.dll') or 'alloc' in name
            or name.startswith('mi_') or name.startswith('_mi_'))


WIDTH = 110  # 名前の表示の幅（総称の引数で長くなる名前を切る）


def clip(name):
    return name if len(name) <= WIDTH else name[:WIDTH - 3] + '...'


def show(title, counter, total, top):
    print(f'\n-- {title}')
    for name, c in counter.most_common(top):
        print(f'{100 * c / total:6.1f}%  {c:6d}  {clip(name)}')


def main():
    args = sys.argv[1:]
    if not args:
        print('usage: analyze.py <file.folded> [--top N] [--incl a,b] [--under name] [--callers name]')
        return 2
    path, top, incl, under, callers = args[0], 25, [], None, None
    i = 1
    while i < len(args):
        if args[i] == '--top':
            top = int(args[i + 1])
        elif args[i] == '--incl':
            incl = [x for x in args[i + 1].split(',') if x]
        elif args[i] == '--under':
            under = args[i + 1]
        elif args[i] == '--callers':
            callers = args[i + 1]
        else:
            print(f'unknown option: {args[i]}')
            return 2
        i += 2
    stacks = load(path)
    if under:
        cut = []
        for frames, c in stacks:
            idx = [k for k, (_, n) in enumerate(frames) if under in n]
            if idx:
                cut.append((frames[idx[-1]:], c))
        stacks = cut
    total = sum(c for _, c in stacks)
    print(f'== {path}  samples (interpreter thread{", under " + under if under else ""}) = {total}')
    if total == 0:
        return 0

    by_cb, leaf = Counter(), Counter()
    for frames, c in stacks:
        last = max((k for k, (m, _) in enumerate(frames) if is_generated(m)), default=None)
        if last is None:
            by_cb['(outside generated code)'] += c
        elif last == len(frames) - 1:
            by_cb['(generated code itself)'] += c
        else:
            by_cb[frames[last + 1][1]] += c
        mod, name = frames[-1]
        if is_generated(mod):
            leaf['(generated code itself)'] += c
        else:
            leaf[('' if mod.lower() == 'arrow.exe' else mod.lower() + '!') + name] += c
    show('from generated code (inclusive)', by_cb, total, top)
    show('self time', leaf, total, top)

    if incl:
        print('\n-- inclusive (name contains)')
        for sub in incl:
            c = sum(cnt for frames, cnt in stacks if any(sub in n for _, n in frames))
            print(f'{100 * c / total:6.1f}%  {c:6d}  {sub}')
    # ヒープの確保・解放（葉が OS のヒープか Rust の確保の入口）
    heap = sum(c for frames, c in stacks if is_heap(*frames[-1]))
    print(f'\n-- heap (self time in ntdll / kernel32 / allocator functions)\n{100 * heap / total:6.1f}%  {heap:6d}')

    if callers:
        cc = Counter()
        for frames, c in stacks:
            idx = [k for k, (_, n) in enumerate(frames) if callers in n]
            if idx:
                k = idx[0]
                cc[frames[k - 1][1] if k > 0 else '(root)'] += c
        show(f'callers of {callers}', cc, total, top)
    return 0


sys.exit(main())
