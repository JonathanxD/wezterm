#!/usr/bin/env python3
import json
import sys
import re

CATEGORIZE = {
    r".el7.x86_64.rpm$": "centos7_rpm",
    r".centos7.rpm$": "centos7_rpm",
    r".el8.x86_64.rpm$": "centos8_rpm",
    r".centos8.rpm$": "centos8_rpm",
    r".el9.x86_64.rpm$": "centos9_rpm",
    r".centos9.rpm$": "centos9_rpm",
    r".fc31.x86_64.rpm$": "fedora31_rpm",
    r".fedora31.rpm$": "fedora31_rpm",
    r".fc32.x86_64.rpm$": "fedora32_rpm",
    r".fedora32.rpm$": "fedora32_rpm",
    r".fc33.x86_64.rpm$": "fedora33_rpm",
    r".fedora33.rpm$": "fedora33_rpm",
    r".fc34.x86_64.rpm$": "fedora34_rpm",
    r".fedora34.rpm$": "fedora34_rpm",
    r".fc35.x86_64.rpm$": "fedora35_rpm",
    r".fedora35.rpm$": "fedora35_rpm",
    r"Debian9.12.deb$": "debian9_deb",
    r"Debian10.deb$": "debian10_deb",
    r"Debian11.deb$": "debian11_deb",
    r"Ubuntu16.04.AppImage$": "ubuntu16_AppImage",
    r"Ubuntu18.04.AppImage$": "ubuntu18_AppImage",
    r"Ubuntu16.04.deb$": "ubuntu16_deb",
    r"^wezterm-\d+-\d+-[a-f0-9]+.deb$": "ubuntu16_deb",
    r"Ubuntu18.04.deb$": "ubuntu18_deb",
    r"Ubuntu20.04.deb$": "ubuntu20_deb",
    r"Ubuntu18.04.tar.xz$": "linux_raw_bin",
    r"^wezterm-\d+-\d+-[a-f0-9]+.tar.xz$": "linux_raw_bin",
    r"src.tar.gz$": "src",
    r"^WezTerm-macos-.*.zip$": "macos_zip",
    r"^WezTerm-windows-.*.zip$": "windows_zip",
    r"^WezTerm-.*.setup.exe$": "windows_exe",
    r"alpine3.12.apk": "alpine3_12_apk",
    r"alpine3.13.apk": "alpine3_13_apk",
    r"alpine3.14.apk": "alpine3_14_apk",
    r"alpine3.15.apk": "alpine3_15_apk",
}


def categorize(rel):
    downloads = {}

    tag_name = "wezterm-%s" % rel["tag_name"]
    for asset in rel["assets"]:
        url = asset["browser_download_url"]
        name = asset["name"]

        for k, v in CATEGORIZE.items():
            if re.search(k, name):
                downloads[v] = (url, name, tag_name)

    return downloads


def pretty(o):
    return json.dumps(o, indent=4, sort_keys=True, separators=(",", ":"))


def build_subst(subst, stable, categorized):
    for (kind, info) in categorized.items():
        if info is None:
            continue
        url, name, dir = info
        kind = f"{kind}_{stable}"
        subst["{{ %s }}" % kind] = url
        subst["{{ %s_asset }}" % kind] = name
        subst["{{ %s_dir }}" % kind] = dir


def load_release_info():
    with open("/tmp/wezterm.releases.json") as f:
        release_info = json.load(f)

    with open("/tmp/wezterm.nightly.json") as f:
        nightly = json.load(f)


    latest = None
    for rel in release_info:
        if rel["prerelease"]:
            continue
        latest = rel
        break

    latest = categorize(latest)
    nightly = categorize(nightly)

    print("latest: ", pretty(latest))
    print("nightly: ", pretty(nightly))

    subst = {}
    build_subst(subst, "stable", latest)
    build_subst(subst, "nightly", nightly)
    print(pretty(subst))

    for name in [
        "install/windows",
        "install/macos",
        "install/linux",
        "install/source",
        "install/freebsd",
    ]:
        with open(f"docs/{name}.markdown", "r") as input:
            with open(f"docs/{name}.md", "w") as output:
                for line in input:
                    for (search, replace) in subst.items():
                        line = line.replace(search, replace)
                    output.write(line)


def main():
    load_release_info()


main()
