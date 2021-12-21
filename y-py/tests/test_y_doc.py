from y_py import YDoc
import pytest


def test_constructor_options():
    # Ensure that valid versions can be called without error
    YDoc()
    with_id = YDoc(1)
    assert with_id.id == 1
    YDoc(1, "utf-8", True)
    YDoc(client_id=2, offset_kind="utf-8", skip_gc=True)
    YDoc(client_id=3)
    YDoc(4, skip_gc=True)

    # Handle encoding string variation
    YDoc(offset_kind="utf8")
    YDoc(offset_kind="utf-8")
    YDoc(offset_kind="UTF-8")
    YDoc(offset_kind="UTF32")

    # Ensure that incorrect encodings throw error
    with pytest.raises(ValueError):
        YDoc(offset_kind="UTF-0xDEADBEEF")
    with pytest.raises(ValueError):
        YDoc(offset_kind="😬")

    
