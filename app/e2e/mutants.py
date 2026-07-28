#!/usr/bin/env python3
"""Guard enumeration for the /find wiring, as applyable mutations (A74).

Every entry is a decision the wiring encodes — not a self-selected list of
edits. `apply` rewrites exactly one occurrence of `find` (asserting it occurs
exactly once, so a drifted source fails loudly instead of silently mutating
nothing) and keeps a byte-identical backup; `restore` puts every backup back.

Usage:  python3 e2e/mutants.py list | apply <id> | restore
"""
import os
import shutil
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
# Backups live outside the tree so a mutation never leaves a stray file in
# app/src, which this node may not modify.
BACKUP_DIR = os.path.join(tempfile.gettempdir(), "find-e2e-mutation-backups")
FUNNEL = "src/components/find/FindFunnel.tsx"
PAGE = "src/app/find/page.tsx"

EFFECT_BODY = """    reconcileUrlAnswers(
      {
        restore: (answers) => dispatch({ type: "restored", answers }),
        request: (answers) => {
          dispatch({ type: "loading" });
          void requestNext(answers, false);
        },
        replaceUrl: (href) => router.replace(href, { scroll: false }),
      },
      answerParam,
      stateRef.current,
    );
"""

# id -> (file, what the guard is, exact text, replacement)
MUTANTS = {
    "G1": (FUNNEL, "the useEffect calling reconcileUrlAnswers", EFFECT_BODY, ""),
    "G2": (
        FUNNEL,
        "`restore` dep -> the reducer",
        '        restore: (answers) => dispatch({ type: "restored", answers }),\n',
        "        restore: () => {},\n",
    ),
    "G3": (
        FUNNEL,
        "`request` dep -> /api/find/next",
        """        request: (answers) => {
          dispatch({ type: "loading" });
          void requestNext(answers, false);
        },
""",
        "        request: () => {},\n",
    ),
    "G4": (
        FUNNEL,
        "`replaceUrl` dep -> router.replace",
        "        replaceUrl: (href) => router.replace(href, { scroll: false }),\n",
        "        replaceUrl: () => {},\n",
    ),
    "G5": (
        FUNNEL,
        "router.push in handleAnswer",
        "    router.push(findFunnelHref(answers), { scroll: false });\n",
        "",
    ),
    "G6": (
        FUNNEL,
        'handleBack "pop" -> router.back()',
        "    if (step === \"pop\") router.back();\n",
        '    if (step === "pop") router.replace(findFunnelHref(answers), { scroll: false });\n',
    ),
    "G7": (
        FUNNEL,
        'handleBack "rewrite" -> router.replace',
        "    else router.replace(findFunnelHref(answers), { scroll: false });\n",
        "    else router.back();\n",
    ),
    "G8": (
        FUNNEL,
        "landingCountRef initialised from the landing URL",
        "    landingCountRef.current = parseFunnelAnswers(answerParam).length;\n",
        "    landingCountRef.current = 0;\n",
    ),
    "G9": (
        PAGE,
        "`resuming` computed from the URL",
        "  const resuming = (await searchParams)[FUNNEL_ANSWERS_PARAM] !== undefined;\n",
        "  const resuming = false;\n",
    ),
    "G10": (
        PAGE,
        "the `resuming ? null :` short-circuit",
        """  const initialResult = resuming
    ? null
    : await fetchNextFindQuestion({ answers: [] }).catch(() => null);
""",
        "  const initialResult = await fetchNextFindQuestion({ answers: [] }).catch(() => null);\n",
    ),
    "G11": (
        PAGE,
        "FUNNEL_ANSWERS_PARAM is the key page.tsx reads",
        "  const resuming = (await searchParams)[FUNNEL_ANSWERS_PARAM] !== undefined;\n",
        '  const resuming = (await searchParams)["b"] !== undefined;\n',
    ),
    "G12": (
        FUNNEL,
        "FUNNEL_ANSWERS_PARAM is the key FindFunnel reads",
        "  const answerParam = useSearchParams().get(FUNNEL_ANSWERS_PARAM);\n",
        '  const answerParam = useSearchParams().get("b");\n',
    ),
    "G13": (
        FUNNEL,
        "the effect re-runs when answerParam changes",
        "  }, [answerParam, requestNext, router]);\n",
        "  }, []);\n",
    ),
    "G14": (
        FUNNEL,
        "page.tsx's initialResult seeds the reducer",
        "  return { ...initialFunnelState, result: initialResult };\n",
        "  return { ...initialFunnelState, result: null };\n",
    ),
    "G15": (
        FUNNEL,
        "handleAnswer asks the engine for the next question",
        """    dispatch({ type: "answered", facet, value });
    void requestNext(answers, false);
""",
        '    dispatch({ type: "answered", facet, value });\n',
    ),
    "G16": (
        FUNNEL,
        "handleBack drops the last answer",
        "    const answers = state.answers.slice(0, -1);\n",
        "    const answers = state.answers.slice(0);\n",
    ),
    "G17": (
        FUNNEL,
        "handleAnswer moves the reducer before the URL",
        '    dispatch({ type: "answered", facet, value });\n',
        "",
    ),
    "G18": (
        FUNNEL,
        'backNavigation "none" early return',
        '    if (step === "none") return;\n',
        "",
    ),
}



# --- mutual exclusion -------------------------------------------------------
#
# Two agents ran a battery against this one file during the run that produced
# it and silently corrupted each other's verdicts (A75, three times over).
# "Announce before touching" is etiquette between parties who cannot see each
# other's writes; this is the enforcement. The guard lives HERE, not only in
# mutation-battery.sh, because the call that actually did the damage was a bare
# `mutants.py restore` typed as a diagnostic — the dangerous operation was the
# one that did not look like a write.
LOCK_DIR = os.path.join(tempfile.gettempdir(), "find-e2e-mutation.lock")
OWNER_FILE = os.path.join(LOCK_DIR, "owner")


def _alive(pid):
    try:
        os.kill(pid, 0)
    except (OSError, ValueError):
        return False
    return True


def _lock_owner():
    """(pid, alive) of the current lock holder, or (None, False)."""
    try:
        with open(OWNER_FILE, encoding="utf-8") as fh:
            pid = int(fh.read().strip())
    except (OSError, ValueError):
        return None, False
    return pid, _alive(pid)


def guard_write(force=False):
    if force:
        return
    pid, alive = _lock_owner()
    if pid is None:
        return
    if not alive:
        shutil.rmtree(LOCK_DIR, ignore_errors=True)
        sys.stderr.write(f"note: cleared stale mutation lock from dead pid {pid}\n")
        return
    if str(pid) == os.environ.get("FIND_E2E_LOCK_OWNER", ""):
        return  # our own battery's child call
    raise SystemExit(
        f"REFUSING: a mutation run holds the lock (pid {pid}).\n"
        f"  app/src is mid-mutation; writing now would corrupt that run's\n"
        f"  verdicts and could strand a mutant in a feature file.\n"
        f"  Wait for it to finish, or if you are certain it is dead:\n"
        f"    rm -rf {LOCK_DIR}\n"
        f"  Override (mean it): --force"
    )


def _path(rel):
    return os.path.join(ROOT, rel)


def _backup(rel):
    return os.path.join(BACKUP_DIR, rel.replace("/", "__"))


def apply(mutant_id, force=False):
    guard_write(force)
    rel, _what, find, repl = MUTANTS[mutant_id]
    path = _path(rel)
    with open(path, encoding="utf-8") as fh:
        src = fh.read()
    count = src.count(find)
    if count != 1:
        raise SystemExit(
            f"{mutant_id}: expected exactly 1 occurrence in {rel}, found {count}. "
            "The source moved — the guard list is stale, not the code."
        )
    os.makedirs(BACKUP_DIR, exist_ok=True)
    shutil.copyfile(path, _backup(rel))
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(src.replace(find, repl, 1))


def restore(force=False):
    guard_write(force)
    restored = 0
    for rel in {m[0] for m in MUTANTS.values()}:
        backup = _backup(rel)
        if os.path.exists(backup):
            shutil.copyfile(backup, _path(rel))
            os.remove(backup)
            restored += 1
    return restored


if __name__ == "__main__":
    cmd = sys.argv[1] if len(sys.argv) > 1 else "list"
    if cmd == "list":
        for key, value in MUTANTS.items():
            print(f"{key}\t{value[0]}\t{value[1]}")
    elif cmd == "apply":
        apply(sys.argv[2], "--force" in sys.argv)
    elif cmd == "restore":
        print(f"restored {restore('--force' in sys.argv)} file(s)")
    else:
        raise SystemExit(f"unknown command {cmd}")
