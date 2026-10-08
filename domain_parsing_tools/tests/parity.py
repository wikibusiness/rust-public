"""Fuzz domain_parsing_tools against the Python code it replaces.

    PYO3_PYTHON=python3.14 uv run --with maturin maturin build --release
    uv run --no-project --python 3.14 --with target/wheels/domain_parsing_tools-*-cp314-*.whl \
        --with tldextract==5.3.2 --with validators==0.35.0 --with idna==3.20 --with regex \
        python tests/parity.py [iterations] [domains.txt]

domains.txt (optional): one real domain per line, each checked as well.
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


def idna_encode(domain: str) -> str | None:
    try:
        return idna.encode(domain).decode()
    except idna.IDNAError:
        return None


FALLBACKS = {"idna_encode": 0}


def rust_idna_encode(domain: str) -> str | None:
    try:
        return dpt.idna_encode(domain)
    except dpt.UnsupportedDomain:
        FALLBACKS["idna_encode"] += 1
        return idna_encode(domain)


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


# What utils/domain.py's extract_domain hands idna.encode: lowercased, mostly LDH.
NON_ASCII = ["\u00e9", "\u00df", "\U0001f600", "\u0628\u064a", "\u05d0\u05d1", "\u0661", "\u200d", "\u200c", "\u00b7", "\u0430", "\u4e2d", "\u0301", "\uff0e", "\u3002"]
DOMAIN_CHARS = "abcz0189" * 8 + "-" * 12 + "." * 6 + "_A" + "".join(map(chr, range(32, 127)))


def random_domain(rng: random.Random) -> str:
    roll = rng.random()
    if roll < 0.3:
        labels = ["".join(rng.choices(DOMAIN_CHARS.replace(".", ""), k=rng.choice([0, 1, 2, 3, 4, 5, 8, 62, 63, 64]))) for _ in range(rng.randint(1, 4))]
        domain = ".".join(labels)
    elif roll < 0.5:
        domain = "".join(rng.choices(DOMAIN_CHARS, k=rng.randint(0, 20)))
    elif roll < 0.65:
        # Total length around idna's 253 (254 with a trailing dot) limit.
        size = rng.choice([63, 62, 61, 10])
        domain = ".".join("a" * size for _ in range(260 // (size + 1) + 1))[: rng.randint(250, 256)].strip(".")
    elif roll < 0.8:
        label = rng.choice(UNICODE_LABELS + ["b\u00fccher", "stra\u00dfe", "\u05e2\u05d1\u05e8\u05d9\u05ea", "\u0627\u0644\u0639\u0631\u0628\u064a\u0629", "a\u200db", "\u0915\u094d\u200d"])
        puny = label.encode("punycode").decode()
        domain = rng.choice(["xn--" + puny, "XN--" + puny, "xn--" + puny + "x", "xn--", "xn--a-", "xn---", "ab--c", "xn--zz"]) + "." + rng.choice(["com", "de", "-x", "a_b"])
    else:
        chars = list(rng.choice(["acme", "a-b", "123", "ab"]))
        for _ in range(rng.randint(1, 3)):
            chars.insert(rng.randint(0, len(chars)), rng.choice(NON_ASCII))
        domain = "".join(chars) + "." + rng.choice(["com", "de", "xn--p1ai"])
    return domain + rng.choice([""] * 8 + [".", ".."])


def check(url: str) -> None:
    for private in (False, True):
        assert fields(dpt.extract(url, private)) == fields(EXTRACTORS[private](url)), (url, private)
    assert fields(dpt.custom_extract(url)) == fields(custom_tldextract(url)), url
    assert dpt.is_domain(url) == bool(validators.domain(url)), url
    assert dpt.is_valid_main_domain(url) == is_valid_main_domain(url), url
    assert rust_decode_idna(url) == decode_idna(url), url
    assert rust_idna_encode(url) == idna_encode(url), url


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
        domain = random_domain(rng)
        assert rust_idna_encode(domain) == idna_encode(domain), domain
    print(f"{iterations} random URLs and domains, 0 mismatches, fallbacks: {FALLBACKS}")

    if len(sys.argv) > 2:
        domains = Path(sys.argv[2]).read_text().splitlines()
        for domain in domains:
            check(domain)
            check(domain.upper())
        print(f"{len(domains)} real domains, 0 mismatches, fallbacks: {FALLBACKS}")


if __name__ == "__main__":
    main()
