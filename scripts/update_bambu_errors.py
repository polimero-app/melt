#!/usr/bin/env python3
"""Regenerate the offline HMS/print-error catalogue from pinned MIT source data."""

import argparse
import gzip
import json
import re
from pathlib import Path
from urllib.request import urlopen

REVISION = "0e027ff135a6d9265cb756d3e246747954c76722"
SOURCE = f"https://raw.githubusercontent.com/greghesp/ha-bambulab/{REVISION}/custom_components/bambu_lab/pybambu/hms_error_text"
OUTPUT = Path(__file__).resolve().parents[1] / "crates/melt-core/data/bambu-errors.json"


def fetch(name):
    with urlopen(f"{SOURCE}/{name}.json.gz", timeout=30) as response:
        return json.loads(gzip.decompress(response.read()))


def variants(entries, code_lengths, links=False):
    normalized = {}
    for code, messages in entries.items():
        key = code.upper()
        if len(key) not in code_lengths or not re.fullmatch(r"[0-9A-F]+", key):
            raise ValueError(f"Unexpected error code: {code}")
        if key in normalized or not isinstance(messages, dict):
            raise ValueError(f"Invalid variants for {code}")
        for message, models in messages.items():
            if not isinstance(models, list) or any(not isinstance(model, str) for model in models):
                raise ValueError(f"Invalid message/models for {code}")
            if links and (not message.startswith("/en/") or ".." in message.split("/")):
                raise ValueError(f"Unexpected wiki path for {code}: {message}")
        normalized[key] = messages
    return normalized


def catalogue():
    result = {
        "source": {
            "repository": "https://github.com/greghesp/ha-bambulab",
            "revision": REVISION,
            "license": "MIT",
            "licenseNotice": OUTPUT.with_suffix(".LICENSE").read_text(),
            "files": ["hms_en.json.gz", "hms_pt-br.json.gz", "wiki_links.json.gz"],
        }
    }
    for locale, source in [("en", "hms_en"), ("pt-BR", "hms_pt-br")]:
        data = fetch(source)
        if set(data) != {"device_hms", "device_error"}:
            raise ValueError(f"Unexpected catalogue sections in {source}")
        result[locale] = {
            "device_hms": variants(data["device_hms"], (16,)),
            "device_error": variants(data["device_error"], (8,)),
        }
    result["wiki"] = variants(fetch("wiki_links"), (8, 16), links=True)
    return (json.dumps(result, ensure_ascii=False, sort_keys=True, indent=2) + "\n").encode()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="verify the checked-in catalogue without changing it")
    args = parser.parse_args()
    generated = catalogue()
    if args.check:
        if not OUTPUT.exists() or OUTPUT.read_bytes() != generated:
            raise SystemExit("Catalogue differs; run scripts/update_bambu_errors.py and review the source revision.")
        print("Offline Bambu error catalogue matches its pinned sources.")
    else:
        OUTPUT.write_bytes(generated)
        print(f"Generated {OUTPUT.relative_to(OUTPUT.parents[3])} ({len(generated):,} bytes).")


if __name__ == "__main__":
    main()
