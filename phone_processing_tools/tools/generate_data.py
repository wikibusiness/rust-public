import importlib.metadata
import importlib.util
import json
import re
import tempfile
import warnings
from pathlib import Path

import phonenumbers as pn
import regex
from grpc_tools import protoc
from phonenumbers.phonemetadata import PhoneMetadata
from stdnum.util import _char_map
from unidecode import unidecode

ROOT = Path(__file__).resolve().parents[1]
REGEX_FIELDS = {
    "national_number_pattern", "pattern", "leading_digits", "international_prefix",
    "national_prefix_for_parsing",
}
REPLACEMENT_FIELDS = {
    "format", "national_prefix_formatting_rule", "domestic_carrier_code_formatting_rule",
    "national_prefix_transform_rule",
}


def copy_metadata(source, destination):
    for name, field in destination.DESCRIPTOR.fields_by_name.items():
        value = getattr(source, name, None)
        if value is None:
            continue
        if field.is_repeated:
            if field.message_type:
                for item in value:
                    copy_metadata(item, getattr(destination, name).add())
            else:
                if name == "leading_digits_pattern":
                    value = [f"^(?:{item})$" for item in value]
                getattr(destination, name).extend(value)
        elif field.message_type:
            copy_metadata(value, getattr(destination, name))
        else:
            if isinstance(value, str):
                if name in REPLACEMENT_FIELDS:
                    value = re.sub(r"\\([0-9]+)", r"$\1", value)
                if name in REGEX_FIELDS:
                    value = f"^(?:{value})$"
            setattr(destination, name, value)


with tempfile.TemporaryDirectory() as directory:
    schema = ROOT / "tools/phonemetadata.proto"
    if protoc.main(["protoc", f"-I{schema.parent}", f"--python_out={directory}", str(schema)]):
        raise RuntimeError("Phone metadata schema compilation failed")
    spec = importlib.util.spec_from_file_location("phonemetadata_pb2", Path(directory) / "phonemetadata_pb2.py")
    if not spec or not spec.loader:
        raise RuntimeError("Phone metadata module could not be loaded")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    metadata = module.PhoneMetadataCollection()
    for region in sorted(pn.SUPPORTED_REGIONS):
        copy_metadata(PhoneMetadata.metadata_for_region(region), metadata.metadata.add())
    for country_code in sorted(pn.COUNTRY_CODES_FOR_NON_GEO_REGIONS):
        copy_metadata(PhoneMetadata.metadata_for_nongeo_region(country_code), metadata.metadata.add())

(ROOT / "data").mkdir(exist_ok=True)
(ROOT / "data/metadata.bin").write_bytes(metadata.SerializeToString())
print(f"Exported phonenumbers {pn.__version__}: {len(metadata.metadata)} territories", flush=True)

transliterations = []
numeric = []
decimal = []
decimal_pattern = regex.compile(r"\p{Nd}\Z")
number_pattern = regex.compile(r"\p{N}\Z")
with warnings.catch_warnings():
    warnings.simplefilter("ignore")
    for codepoint in range(128, 0x110000):
        if 0xD800 <= codepoint <= 0xDFFF:
            continue
        character = chr(codepoint)
        value = unidecode(character)
        if value:
            transliterations.append((codepoint, value))
        if number_pattern.fullmatch(character):
            numeric.append(codepoint)
        if decimal_pattern.fullmatch(character):
            decimal.append(codepoint)

versions = {name: importlib.metadata.version(name) for name in ["phonenumbers", "Unidecode", "python-stdnum", "regex"]}
data = {
    "versions": versions,
    "transliterations": dict(transliterations),
    "symbols": {ord(character): value for character, value in sorted(_char_map.items())},
    "numeric": numeric,
    "decimal": decimal,
}
(ROOT / "data/normalization.json").write_text(json.dumps(data, ensure_ascii=True, separators=(",", ":")) + "\n")
print(f"Exported normalization: {len(transliterations)} transliterations, {len(numeric)} numeric characters", flush=True)
