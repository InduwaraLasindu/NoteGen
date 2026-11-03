"""Bounded, read-only executable diagnostics. Never substitutes for hash gating."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import struct
import zipfile

LIMIT = 250_000_000


def sha(data):
    return hashlib.sha256(data).hexdigest()


def span(data, offset, size):
    if offset < 0 or size < 0 or offset + size > len(data):
        raise ValueError("PE range outside executable")
    return data[offset:offset + size]


def pe(data):
    if span(data, 0, 2) != b"MZ":
        raise ValueError("Missing DOS header")
    nt = struct.unpack("<I", span(data, 60, 4))[0]
    if span(data, nt, 4) != b"PE\0\0":
        raise ValueError("Missing PE signature")
    machine, count, timestamp, _, _, optional_size, flags = struct.unpack(
        "<HHIIIHH", span(data, nt + 4, 20))
    if not 1 <= count <= 96:
        raise ValueError("PE section count exceeds bound")
    optional = nt + 24
    header = span(data, optional, optional_size)
    magic = struct.unpack("<H", span(header, 0, 2))[0]
    directory = {0x20B: 112, 0x10B: 96}.get(magic)
    if directory is None:
        raise ValueError("Unsupported PE optional header")
    sections = []
    for index in range(count):
        raw = span(data, optional + optional_size + index * 40, 40)
        name, virtual_size, rva, size, offset = struct.unpack("<8sIIII", raw[:24])
        sections.append({"name": name.rstrip(b"\0").decode("ascii", "replace"),
                         "rva": rva, "virtual_bytes": virtual_size,
                         "file_offset": offset, "bytes": size,
                         "sha256": sha(span(data, offset, size))})

    def rva_offset(address, size):
        for section in sections:
            delta = address - section["rva"]
            if 0 <= delta and delta + size <= section["bytes"]:
                return section["file_offset"] + delta
        raise ValueError("PE debug RVA outside file-backed sections")

    debug = []
    directory_count = struct.unpack("<I", span(header, directory - 4, 4))[0]
    if directory_count > 6:
        address, size = struct.unpack("<II", span(header, directory + 6 * 8, 8))
        if size % 28 or size > 64 * 28:
            raise ValueError("PE debug directory exceeds bound")
        if size:
            base = rva_offset(address, size)
            for index in range(size // 28):
                _, stamp, major, minor, kind, length, _, pointer = struct.unpack(
                    "<IIHHIIII", span(data, base + index * 28, 28))
                if length > 1_000_000:
                    raise ValueError("PE debug payload exceeds bound")
                payload = span(data, pointer, length)
                item = {"type": kind, "timestamp": stamp, "major": major,
                        "minor": minor, "bytes": length, "sha256": sha(payload),
                        "prefix_hex": payload[:128].hex()}
                if kind == 2 and payload.startswith(b"RSDS") and length >= 24:
                    item.update(rsds_guid_hex=payload[4:20].hex(),
                                age=struct.unpack("<I", payload[20:24])[0],
                                pdb=payload[24:].split(b"\0", 1)[0][:512].decode("utf-8", "replace"))
                debug.append(item)
    return {"machine": machine, "coff_timestamp": timestamp, "flags": flags,
            "sections": sections, "debug": debug}


def paths(data):
    result = []
    patterns = [("ascii", rb"[\x20-\x7e]{5,}"),
                ("utf-16le", rb"(?:[\x20-\x7e]\x00){5,}")]
    for encoding, pattern in patterns:
        for match in re.finditer(pattern, data):
            value = match.group().decode(encoding)
            if not any(token in value.lower() for token in
                       ("/build", "/cargo", "/notegen", "\\target\\", "/target/",
                        "reproduction-target", ".pdb", ":\\", ":/")):
                continue
            result.append({"offset": match.start(), "encoding": encoding,
                           "text": value[:512], "truncated": len(value) > 512,
                           "string_sha256": sha(match.group())})
            if len(result) == 32:
                return result
    return result


def describe(data):
    value = {"bytes": len(data), "sha256": sha(data), "path_strings": paths(data)}
    try:
        value["pe"] = pe(data)
    except (ValueError, struct.error) as error:
        value["pe_error"] = str(error)
    return value


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--reference-archive", type=Path, required=True)
    parser.add_argument("--candidate-directory", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    report = {"schema": 1, "scope": "Read-only bounded diagnostics; executable bytes are never normalized or changed. Strict SHA256 comparison remains required.", "subjects": []}
    with zipfile.ZipFile(args.reference_archive) as archive:
        for name in ("NoteGen.exe", "notegen-cli.exe"):
            matches = [entry for entry in archive.infolist() if entry.filename == "NoteGen/" + name]
            if len(matches) != 1 or matches[0].file_size > LIMIT:
                raise ValueError("Missing, duplicate or oversized reference executable")
            reference = archive.read(matches[0])
            path = args.candidate_directory / name
            if path.stat().st_size > LIMIT:
                raise ValueError("Oversized candidate executable")
            with path.open("rb") as file:
                candidate = file.read(LIMIT + 1)
            if len(candidate) > LIMIT:
                raise ValueError("Candidate grew beyond diagnostic bound")
            first = []
            for index, (left, right) in enumerate(zip(reference, candidate)):
                if left != right:
                    first.append(index)
                    if len(first) == 32:
                        break
            if len(first) < 32 and len(reference) != len(candidate):
                begin = min(len(reference), len(candidate))
                first.extend(range(begin, begin + min(32 - len(first), abs(len(reference) - len(candidate)))))
            subject = {"name": name, "matched": reference == candidate,
                       "first_32_difference_offsets": first,
                       "reference": describe(reference), "candidate": describe(candidate)}
            report["subjects"].append(subject)
            print(f"{name}: matched={subject['matched']} reference={subject['reference']['sha256']} candidate={subject['candidate']['sha256']}")
    report["matched"] = all(subject["matched"] for subject in report["subjects"])
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
