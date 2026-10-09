import json
from pathlib import Path

import phone_processing_tools as native

root = Path(__file__).resolve().parents[1]
data = json.loads((root / "data/normalization.json").read_text())
assert native.metadata_versions() == data["versions"]
texts = ["Fax: +４５ ３３６６ ３３６６", "日本語", "① / ¼", "\x1ctel\x1d+45\x1e33663366", "((123456))"]
assert native.preprocess_texts(texts) == ["fax: +45 3366 3366", "ri ben yu", "1 1/4", "tel +45 33663366", "(123456)"]
assert native.format_numbers(["+453 366 3366", "11111"], True) == ["+45 33 66 33 66", None]
phone = native.PhoneRecord("0251987770", ["0251987770"], "phone", False, ["web"], None, None)
result = native.validate_phones([phone], ["de"], True)
assert [(p.number, p.country) for p in result] == [("+49 251 987770", "de")]
assert phone.number == "0251987770"
assert [(p.number, p.kind) for p in native.extract_text_phones("Fax: +4533663366; 2015-2020", True, 2027)] == [("+4533663366", "fax")]
print("Native phone binding checks passed", flush=True)
