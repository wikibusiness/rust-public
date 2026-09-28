"""Fuzz domain_parsing_tools against the Python code it replaces.

    PYO3_PYTHON=python3.14 uv run --with maturin maturin build --release
    uv run --no-project --python 3.14 --with target/wheels/domain_parsing_tools-*-cp314-*.whl \
        --with tldextract==5.3.2 --with validators==0.35.0 --with idna==3.20 --with regex \
        python tests/parity.py [iterations]
"""

import random
import sys
from pathlib import Path

import domain_parsing_tools as dpt
import idna
import regex
import validators
from tldextract import TLDExtract
from tldextract.tldextract import ExtractResult, _decode_punycode

PSL = Path(__file__).parent.parent / "public_suffix_list.dat"
EXTRACTORS = {
    private: TLDExtract(
        suffix_list_urls=[PSL.resolve().as_uri()],
        cache_dir=None,
        fallback_to_snapshot=False,
        include_psl_private_domains=private,
    )
    for private in (False, True)
}
STOP_SUBDOMAIN_RE = regex.compile(r"^ww[w\d]\d?(\.|$)")


# platform3's backend-python/common/utils/domain.py, before the port.
def custom_tldextract(url: str) -> ExtractResult:
    extract_result = EXTRACTORS[True](url)
    is_stop_subdomain = bool(STOP_SUBDOMAIN_RE.match(extract_result.domain))
    if (
        is_stop_subdomain or not extract_result.top_domain_under_public_suffix
    ) and len(extract_result.suffix.split(".")) >= 2:
        subdomain = extract_result.domain if is_stop_subdomain else extract_result.subdomain
        suffix_split = extract_result.suffix.split(".")
        suffix = ".".join(suffix_split[1:])
        return ExtractResult(
            subdomain=subdomain,
            domain=suffix_split[0],
            suffix=suffix,
            is_private=extract_result.is_private,
            registry_suffix=suffix,
        )
    return extract_result


def is_valid_main_domain(domain: str) -> bool:
    if not validators.domain(domain):
        return False
    return domain == custom_tldextract(domain).top_domain_under_public_suffix


def decode_idna(domain: str) -> str | None:
    if "_" in domain:
        return domain
    try:
        return idna.decode(domain)
    except UnicodeError:
        return None


def rust_decode_idna(domain: str) -> str | None:
    try:
        return dpt.decode_idna(domain)
    except dpt.UnsupportedDomain:
        return decode_idna(domain)


def fields(result) -> tuple:
    return (
        result.subdomain,
        result.domain,
        result.suffix,
        result.is_private,
        result.registry_suffix,
        result.top_domain_under_public_suffix,
        result.fqdn,
    )


SUFFIXES = sorted(EXTRACTORS[True].tlds)
UNICODE_LABELS = sorted({l for s in SUFFIXES for l in s.split(".") if not l.isascii()})
CHARS = "abcxyzABCXYZ0189-" * 6 + "._ *!:@/?#[]%\t\n\x1c\xa0\u3000\u3002\uff0e\u0663\u00fc\u00dc\u0130\u03a3\u212a"


def random_label(rng: random.Random) -> str:
    roll = rng.random()
    if roll < 0.08:
        return rng.choice(["www", "WWW", "ww1", "ww12", "wwwx", "ww\u0663", "www\n", "xn--", "Xn--bcher-kva", "xn--abc-"])
    if roll < 0.12:
        return "a" * rng.choice([62, 63, 64])
    if roll < 0.2:
        label = rng.choice(UNICODE_LABELS)
        return rng.choice([label, label.upper(), "xn--" + label.encode("punycode").decode(), "XN--" + label.encode("punycode").decode().upper()])
    return "".join(rng.choices(CHARS, k=rng.randint(0, 8)))


def random_url(rng: random.Random) -> str:
    roll = rng.random()
    if roll < 0.03:
        host = rng.choice(["1.2.3.4", "01.2.3.4", "256.1.1.1", "1.2.3", "\u0663.2.3.4", "[::1]", "[fe80::1%eth0]", "[::ffff:1.2.3.4]", "[zz]", "[]", "[::1"])
    else:
        suffix = rng.choice(SUFFIXES).replace("*", rng.choice(["x", "www", "*", "xn--p1ai"])).lstrip("!")
        if rng.random() < 0.2:
            suffix = suffix.upper()
        host = ".".join([*(random_label(rng) for _ in range(rng.choice([0, 1, 1, 1, 2, 3]))), suffix])
        if rng.random() < 0.03:
            host = "a." * rng.randint(120, 130) + host
        host += rng.choice([""] * 10 + [".", "..", "\u3002", " ", "\t"])
    if rng.random() < 0.3:
        host = rng.choice(["http://", "https://", "//", "ftp:", "a+b://", "1a://", "h\u00fc://", " http://", "mailto:"]) + host
    if rng.random() < 0.1:
        host = host.replace("//", "//user:pw@", 1) if "//" in host else "u@" + host
    if rng.random() < 0.2:
        host += rng.choice([":8080", "/path", "?q=1", "#frag", ":80/a@b", "/a.b.com"])
    return host


def check(url: str) -> None:
    for private in (False, True):
        assert fields(dpt.extract(url, private)) == fields(EXTRACTORS[private](url)), (url, private)
    assert fields(dpt.custom_extract(url)) == fields(custom_tldextract(url)), url
    assert dpt.is_domain(url) == bool(validators.domain(url)), url
    assert dpt.is_valid_main_domain(url) == is_valid_main_domain(url), url
    assert rust_decode_idna(url) == decode_idna(url), url


def main() -> None:
    assert dpt.private_suffixes() == set(EXTRACTORS[True].tlds) - set(EXTRACTORS[False].tlds)
    # decode_punycode skips idna's check_label: only sound while every non-ASCII
    # suffix label survives it and no suffix label is itself an A-label.
    for label in UNICODE_LABELS:
        assert _decode_punycode("xn--" + label.encode("punycode").decode()) == label, label
    assert not any(l.startswith("xn--") for s in SUFFIXES for l in s.split("."))

    for value in [None, 0, 1.5, b"acme.com", ["acme.com"]]:
        assert dpt.is_domain(value) == bool(validators.domain(value)) == is_valid_main_domain(value) == dpt.is_valid_main_domain(value) == False, value

    iterations = int(sys.argv[1]) if len(sys.argv) > 1 else 200_000
    rng = random.Random(0)
    for _ in range(iterations):
        check(random_url(rng))
    print(f"{iterations} random inputs, 0 mismatches")


if __name__ == "__main__":
    main()
