#!/usr/bin/env python3
"""Copies libmpv and every library it needs, other than the system's, into an application bundle's Frameworks folder and
points them at each other there.

    bundle-dylibs.py <libmpv.dylib> <Frameworks folder>

Prints the newest macOS version any of the copied libraries was built for: the app cannot run on anything older.
"""
import os
import shutil
import subprocess
import sys

SYSTEM = ("/usr/lib/", "/System/")


def run(*args):
    return subprocess.run(args, check=True, capture_output=True, text=True).stdout


def references(path):
    """The libraries `path` names, without its own name."""
    own = run("otool", "-D", path).splitlines()[1:]
    names = [line.strip().split(" (compatibility")[0] for line in run("otool", "-L", path).splitlines()[1:]]
    return [name for name in names if name not in own]


def rpaths(path):
    lines = run("otool", "-l", path).splitlines()
    return [lines[i + 2].split()[1] for i, line in enumerate(lines) if line.strip() == "cmd LC_RPATH"]


def resolve(reference, source):
    """The file a reference in the library at `source` stands for."""
    here = os.path.dirname(source)
    if reference.startswith("@rpath/"):
        candidates = [os.path.join(p.replace("@loader_path", here), reference[len("@rpath/"):]) for p in rpaths(source)]
        candidates.append(os.path.join(here, reference[len("@rpath/"):]))
    else:
        candidates = [reference.replace("@loader_path", here)]
    for candidate in candidates:
        if os.path.isfile(candidate):
            return os.path.realpath(candidate)
    sys.exit(f"cannot find {reference}, needed by {source}")


def minimum_version(path):
    for line in run("vtool", "-show-build", path).splitlines():
        if line.split()[:1] == ["minos"]:
            return tuple(int(part) for part in line.split()[1].split("."))
    return (0,)


def main():
    root, folder = os.path.realpath(sys.argv[1]), sys.argv[2]
    os.makedirs(folder, exist_ok=True)
    # Real file -> the name it gets in the bundle. libmpv keeps the name the app looks for.
    names = {root: "libmpv.2.dylib"}
    changes = {}
    todo = [root]
    while todo:
        source = todo.pop()
        changes[source] = []
        for reference in references(source):
            if reference.startswith(SYSTEM):
                continue
            target = resolve(reference, source)
            if target == source:
                continue
            if target not in names:
                name = os.path.basename(reference)
                if name in names.values():
                    sys.exit(f"two different libraries are called {name}")
                names[target] = name
                todo.append(target)
            changes[source].append((reference, names[target]))

    newest = (0,)
    for source, name in names.items():
        copy = os.path.join(folder, name)
        shutil.copyfile(source, copy)
        os.chmod(copy, 0o755)
        edit = ["-id", f"@rpath/{name}"]
        for reference, other in changes[source]:
            edit += ["-change", reference, f"@loader_path/{other}"]
        for path in set(rpaths(copy)):
            edit += ["-delete_rpath", path]
        run("install_name_tool", *edit, copy)
        newest = max(newest, minimum_version(copy))

    for name in names.values():
        outside = [r for r in references(os.path.join(folder, name)) if not r.startswith(SYSTEM + ("@loader_path/",))]
        if outside:
            sys.exit(f"{name} still points outside the bundle: {outside}")
    print(f"{len(names)} libraries bundled", file=sys.stderr)
    print(".".join(str(part) for part in newest))


if __name__ == "__main__":
    main()
