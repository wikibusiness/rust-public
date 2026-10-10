import importlib
import json
import sys
import tempfile
from pathlib import Path

assert sys.version_info >= (3, 14)
assert sys.version_info.releaselevel == "final" or sys.version_info == (
    3, 15, 0, "candidate", 3
)

name = sys.argv[1]
module = importlib.import_module(name)

if name == "html_parsing_tools":
    assert module.get_href_attributes('<a href="https://example.com">Example</a>') == [
        "https://example.com"
    ]
    assert module.split_sentence("Hello world", split_words={"world"}) == ["Hello"]
elif name == "human_name_parser":
    result = module.parse_name("Ada Lovelace")
    assert result["given_name"] == "Ada"
    assert result["surname"] == "Lovelace"
    assert module.parse_name("") == {}
elif name == "lzstring_optimized":
    for value in ("", "Hello, world", "Hej 🌊 — こんにちは"):
        assert module.decompress_from_base64(module.compress_to_base64(value)) == value
    try:
        module.decompress_from_base64("invalid")
    except TypeError:
        pass
    else:
        raise AssertionError("Invalid compressed input was accepted")
elif name == "color_palette_extract":
    assert module.get_hex_from_rgb(255.0, 0.0, 0.0) == "#ff0000"
    assert module.get_hsl_from_rgb(255.0, 0.0, 0.0) == [0.0, 100.0, 50.0]
    try:
        module.extract_from_bytes(b"not an image", False, 1.0, 1.0)
    except module.ImageLoadError:
        pass
    else:
        raise AssertionError("Invalid image input was accepted")
elif name == "domain_parsing_tools":
    result = module.extract("https://www.example.co.uk/path")
    assert (result.subdomain, result.domain, result.suffix) == ("www", "example", "co.uk")
    assert module.is_domain("example.com")
    assert not module.is_domain(None)
    assert module.idna_encode("example.com") == "example.com"
    assert module.decode_idna("EXAMPLE.COM") == "example.com"
    try:
        module.idna_encode("münich.de")
    except module.UnsupportedDomain:
        pass
    else:
        raise AssertionError("Non-ASCII domain did not request the Python fallback")
elif name == "tech_detector":
    detector = module.TechDetector(
        json.dumps({"Example": {"html": "example", "implies": "Dependency"}}).encode()
    )
    assert detector.detect_text_key("html", [b"example"]) == ["Example"]
    assert set(detector.detect_full([b"example"], [], [], [], [])) == {
        "Example",
        "Dependency",
    }
elif name == "phone_processing_tools":
    assert module.preprocess_texts(["Fax: +４５ ３３６６ ３３６６"]) == [
        "fax: +45 3366 3366"
    ]
    assert module.format_numbers(["+4533663366", "invalid"], True) == [
        "+45 33 66 33 66", None
    ]
elif name == "python_import_graph":
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        source = root / "main.py"
        dependency = root / "dependency.py"
        source.write_text("lazy import dependency\nlazy from dependency import value\n")
        dependency.write_text("value = 1\n")
        paths, roots = [str(source)], [str(root)]
        expected = {str(source): ([str(dependency)], False)}
        assert module.resolve_imports(paths, roots) == expected
        assert module.ImportResolver().resolve(paths, roots) == expected
else:
    raise AssertionError(f"No smoke check for {name}")

print(f"{name}: wheel smoke check passed on Python {sys.version.split()[0]}")
