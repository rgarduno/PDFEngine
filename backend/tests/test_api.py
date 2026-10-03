"""Integration tests for the PDFEngine REST API."""

import io
import os
import threading

os.environ["PDFENGINE_API_KEYS"] = "pdfengine-test-key-0001,pdfengine-test-key-0002"

import pytest
from fastapi.testclient import TestClient
from starlette.websockets import WebSocketDisconnect

from app.audit import MAX_AUDIT_EVENTS, clear_events, events_for, record
from app.auth import (
    DOCUMENT_SESSIONS,
    _ws_tickets,
    adopt_subject,
    authenticate_header,
    clear_document_locks,
    document_mutation,
    document_mutations,
    reset_subject,
)
from app.main import app, get_document_overview, rotate_page_endpoint
from app.models import RotatePageRequest

client = TestClient(app, headers={"Authorization": "Bearer pdfengine-test-key-0001"})
other_client = TestClient(app, headers={"Authorization": "Bearer pdfengine-test-key-0002"})
anonymous = TestClient(app)


@pytest.fixture(autouse=True)
def _clear_document_sessions():
    """Drop leftover sessions so the default cap of 32 does not depend on order."""
    DOCUMENT_SESSIONS.clear()
    _ws_tickets.clear()
    clear_document_locks()
    clear_events()
    yield
    DOCUMENT_SESSIONS.clear()
    _ws_tickets.clear()
    clear_document_locks()
    clear_events()


def _upload_minimal(name: str = "sample.pdf"):
    pdf = create_minimal_pdf_bytes()
    response = client.post(
        "/api/documents/upload",
        files={"file": (name, io.BytesIO(pdf), "application/pdf")},
    )
    return pdf, response


def create_minimal_pdf_bytes() -> bytes:
    """Generates a minimal valid ISO 32000 PDF document byte sequence."""
    pdf = bytearray()
    pdf.extend(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n")

    off1 = len(pdf)
    pdf.extend(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n")

    off2 = len(pdf)
    pdf.extend(b"2 0 obj\n<< /Type /Pages /Kids [ 3 0 R ] /Count 1 >>\nendobj\n")

    off3 = len(pdf)
    pdf.extend(
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [ 0 0 612 792 ] /Contents 4 0 R >>\nendobj\n"
    )

    off4 = len(pdf)
    stream_content = (
        b"BT\n/F1 14 Tf\n50 700 Tm\n(Contract Agreement Terms) Tj\n0 -16 Td\n(All clauses active.) Tj\nET\n"
    )
    pdf.extend(f"4 0 obj\n<< /Length {len(stream_content)} >>\nstream\n".encode())
    pdf.extend(stream_content)
    pdf.extend(b"endstream\nendobj\n")

    xref_offset = len(pdf)
    pdf.extend(b"xref\n0 5\n0000000000 65535 f \n")
    pdf.extend(f"{off1:010} 00000 n \n".encode())
    pdf.extend(f"{off2:010} 00000 n \n".encode())
    pdf.extend(f"{off3:010} 00000 n \n".encode())
    pdf.extend(f"{off4:010} 00000 n \n".encode())

    pdf.extend(b"trailer\n<< /Size 5 /Root 1 0 R >>\n")
    pdf.extend(f"startxref\n{xref_offset}\n%%EOF\n".encode())

    return bytes(pdf)


def create_active_content_pdf_bytes() -> bytes:
    """PDF with a JavaScript open action, a script URI, and one https link."""
    visible = b"BT (JavaScript is a visible word.) Tj ET\n"
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R /OpenAction 5 0 R >>",
        b"<< /Type /Pages /Kids [ 3 0 R ] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [ 0 0 612 792 ] /Contents 4 0 R /Annots [ 6 0 R 7 0 R ] >>",
        f"<< /Length {len(visible)} >>\nstream\n".encode() + visible + b"endstream",
        b"<< /S /JavaScript /JS (app.alert(1)) >>",
        b"<< /Type /Annot /Subtype /Link /Rect [ 0 0 10 10 ] /A << /S /URI /URI (https://example.com/docs) >> >>",
        b"<< /Type /Annot /Subtype /Link /Rect [ 20 0 30 10 ] /A << /S /URI /URI (javascript:alert(1)) >> >>",
    ]
    pdf = bytearray(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n")
    offsets = []
    for index, body in enumerate(objects, start=1):
        offsets.append(len(pdf))
        pdf.extend(f"{index} 0 obj\n".encode())
        pdf.extend(body)
        pdf.extend(b"\nendobj\n")
    xref = len(pdf)
    pdf.extend(f"xref\n0 {len(objects) + 1}\n".encode())
    pdf.extend(b"0000000000 65535 f \n")
    for offset in offsets:
        pdf.extend(f"{offset:010d} 00000 n \n".encode())
    pdf.extend(
        f"trailer\n<< /Size {len(objects) + 1} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode()
    )
    return bytes(pdf)


def test_export_strips_active_actions_and_keeps_https_link():
    upload = client.post(
        "/api/documents/upload",
        files={"file": ("active.pdf", create_active_content_pdf_bytes(), "application/pdf")},
    )
    assert upload.status_code == 200
    doc_id = upload.json()["document_id"]
    exported = client.get(f"/api/documents/{doc_id}/export")
    assert exported.status_code == 200
    body = exported.content
    for marker in (
        b"/OpenAction",
        b"/JavaScript",
        b"/JS",
        b"javascript:",
        b"app.alert(1)",
    ):
        assert marker not in body
    assert b"https://example.com/docs" in body
    assert b"JavaScript is a visible word." in body


def test_health_endpoint():
    response = client.get("/api/health")
    assert response.status_code == 200
    data = response.json()
    assert data["status"] == "healthy"
    assert data["version"] == "0.1.0"


def test_upload_over_the_configured_limit_is_rejected(monkeypatch):
    monkeypatch.setenv("PDFENGINE_MAX_UPLOAD_BYTES", "64")
    body = b"%PDF-1.7\n" + b"\x00" * 80
    response = client.post(
        "/api/documents/upload",
        files={"file": ("oversized.pdf", io.BytesIO(body), "application/pdf")},
    )
    assert response.status_code == 413
    assert response.json()["detail"] == "Upload exceeds the configured size limit."
    assert DOCUMENT_SESSIONS == {}


def test_second_upload_hits_the_session_cap(monkeypatch):
    monkeypatch.setenv("PDFENGINE_MAX_SESSIONS", "1")
    _, first = _upload_minimal("first.pdf")
    assert first.status_code == 200
    session = DOCUMENT_SESSIONS[first.json()["document_id"]]
    assert "raw_bytes" not in session
    assert session["byte_size"] == len(create_minimal_pdf_bytes())

    _, second = _upload_minimal("second.pdf")
    assert second.status_code == 429
    assert second.json()["detail"] == "Document session capacity exceeded."


def test_upload_rejects_when_the_retained_budget_is_smaller_than_the_file(monkeypatch):
    monkeypatch.setenv("PDFENGINE_MAX_RETAINED_BYTES", "32")
    pdf, response = _upload_minimal("budget.pdf")
    assert len(pdf) > 32
    assert response.status_code == 429
    assert DOCUMENT_SESSIONS == {}


def test_expired_session_is_not_found_and_frees_a_slot(monkeypatch):
    monkeypatch.setenv("PDFENGINE_MAX_SESSIONS", "1")
    _, uploaded = _upload_minimal("expiring.pdf")
    assert uploaded.status_code == 200
    doc_id = uploaded.json()["document_id"]
    DOCUMENT_SESSIONS[doc_id]["expires_at"] = 0

    missing = client.get(f"/api/documents/{doc_id}/pages/1/scenegraph")
    assert missing.status_code == 404
    assert missing.json()["detail"] == "Document session not found."
    assert doc_id not in DOCUMENT_SESSIONS

    _, again = _upload_minimal("after-expiry.pdf")
    assert again.status_code == 200


def test_split_rejects_when_the_new_part_would_exceed_the_session_cap(monkeypatch):
    monkeypatch.setenv("PDFENGINE_MAX_SESSIONS", "1")
    _, uploaded = _upload_minimal("source.pdf")
    assert uploaded.status_code == 200
    doc_id = uploaded.json()["document_id"]

    response = client.post(
        f"/api/documents/{doc_id}/split",
        json={"page_indices": [0]},
    )
    assert response.status_code == 429
    assert response.json()["detail"] == "Document session capacity exceeded."
    assert list(DOCUMENT_SESSIONS) == [doc_id]


def test_optimize_keeps_only_the_optimized_buffer(monkeypatch):
    monkeypatch.setenv("PDFENGINE_MAX_SESSIONS", "8")
    _, uploaded = _upload_minimal("optimize_cap.pdf")
    assert uploaded.status_code == 200
    doc_id = uploaded.json()["document_id"]

    response = client.post(
        f"/api/documents/{doc_id}/optimize",
        json={
            "remove_unused": True,
            "pack_object_streams": True,
            "recompress_flate": True,
            "deduplicate_streams": True,
            "max_objects_per_stream": 50,
        },
    )
    assert response.status_code == 200
    session = DOCUMENT_SESSIONS[doc_id]
    assert "raw_bytes" not in session
    assert "optimized_bytes" in session
    assert session["byte_size"] == len(session["optimized_bytes"])


def test_upload_invalid_file_extension():
    response = client.post(
        "/api/documents/upload",
        files={"file": ("test.txt", b"plain text content", "text/plain")},
    )
    assert response.status_code == 400
    assert "Only PDF" in response.json()["detail"]


def test_upload_corrupted_pdf_header():
    response = client.post(
        "/api/documents/upload",
        files={"file": ("corrupted.pdf", b"NOT_A_PDF_STREAM", "application/pdf")},
    )
    assert response.status_code == 400
    assert "not a valid PDF" in response.json()["detail"]


def test_document_lifecycle_upload_inspect_edit_and_export():
    pdf_bytes = create_minimal_pdf_bytes()

    # 1. Upload Document
    upload_resp = client.post(
        "/api/documents/upload",
        files={"file": ("agreement.pdf", io.BytesIO(pdf_bytes), "application/pdf")},
    )
    assert upload_resp.status_code == 200
    doc_data = upload_resp.json()
    doc_id = doc_data["document_id"]
    assert doc_data["filename"] == "agreement.pdf"
    assert doc_data["page_count"] == 1

    # 2. Retrieve Page SceneGraph
    scenegraph_resp = client.get(f"/api/documents/{doc_id}/pages/1/scenegraph")
    assert scenegraph_resp.status_code == 200
    sg_data = scenegraph_resp.json()
    assert sg_data["page_number"] == 1
    assert len(sg_data["paragraphs"]) >= 1

    target_para = sg_data["paragraphs"][0]
    assert "Contract Agreement" in target_para["text"]
    assert target_para["bbox"]["width"] > 0
    assert target_para["bbox"]["height"] > 0

    # 3. Perform Surgical In-Place Edit
    edit_payload = {"new_text": "Amended Agreement Terms Approved by Legal."}
    edit_resp = client.post(
        f"/api/documents/{doc_id}/pages/1/edit/{target_para['id']}",
        json=edit_payload,
    )
    assert edit_resp.status_code == 200
    edit_data = edit_resp.json()
    assert edit_data["success"] is True
    assert "Amended Agreement" in edit_data["updated_text"]

    # 4. Verify SceneGraph reflection after edit
    updated_sg = client.get(f"/api/documents/{doc_id}/pages/1/scenegraph").json()
    assert "Amended Agreement" in updated_sg["paragraphs"][0]["text"]

    # 5. Export Modified PDF
    export_resp = client.get(f"/api/documents/{doc_id}/export")
    assert export_resp.status_code == 200
    assert export_resp.headers["content-type"] == "application/pdf"
    assert (
        export_resp.headers["content-disposition"]
        == 'attachment; filename="document_edited.pdf"'
    )
    assert len(export_resp.content) > 0
    assert export_resp.content.startswith(b"%PDF-")
    assert b"Amended Agreement" in export_resp.content


def test_websocket_realtime_reflow():
    pdf_bytes = create_minimal_pdf_bytes()
    upload_resp = client.post(
        "/api/documents/upload",
        files={"file": ("stream_doc.pdf", io.BytesIO(pdf_bytes), "application/pdf")},
    )
    assert upload_resp.status_code == 200
    doc_id = upload_resp.json()["document_id"]

    # Retrieve paragraph ID
    sg_resp = client.get(f"/api/documents/{doc_id}/pages/1/scenegraph")
    para_id = sg_resp.json()["paragraphs"][0]["id"]

    # Connect to WebSocket reflow channel with a single-use ticket.
    ticket = client.post("/api/auth/ws-ticket").json()["ticket"]
    with client.websocket_connect(
        f"/ws/documents/{doc_id}/pages/1/reflow?ticket={ticket}"
    ) as websocket:
        websocket.send_json({
            "paragraph_id": para_id,
            "text": "Live WebSocket streaming reflow test verification string.",
        })
        response = websocket.receive_json()
        assert response["status"] == "ok"
        assert response["paragraph_id"] == para_id
        assert "reflow test" in response["text"]
        assert "Live WebSocket" in response["text"]
        assert response["line_count"] >= 1
        assert response["bbox"]["width"] > 0

def create_pdf_with_embedded_font_bytes() -> bytes:
    """Generates a PDF containing an embedded TrueType font descriptor and stream."""
    pdf = bytearray()
    pdf.extend(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n")

    off1 = len(pdf)
    pdf.extend(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n")

    off2 = len(pdf)
    pdf.extend(b"2 0 obj\n<< /Type /Pages /Kids [ 3 0 R ] /Count 1 >>\nendobj\n")

    off3 = len(pdf)
    pdf.extend(
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [ 0 0 612 792 ] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>\nendobj\n"
    )

    off4 = len(pdf)
    stream_content = b"BT\n/F1 12 Tf\n50 700 Tm\n(Testing font extraction) Tj\nET\n"
    pdf.extend(f"4 0 obj\n<< /Length {len(stream_content)} >>\nstream\n".encode())
    pdf.extend(stream_content)
    pdf.extend(b"endstream\nendobj\n")

    off5 = len(pdf)
    pdf.extend(
        b"5 0 obj\n<< /Type /Font /Subtype /TrueType /BaseFont /CustomEmbeddedFont /FontDescriptor 6 0 R >>\nendobj\n"
    )

    off6 = len(pdf)
    pdf.extend(
        b"6 0 obj\n<< /Type /FontDescriptor /FontName /CustomEmbeddedFont /FontFile2 7 0 R >>\nendobj\n"
    )

    off7 = len(pdf)
    font_bytes = b"\x00\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00"
    pdf.extend(f"7 0 obj\n<< /Length {len(font_bytes)} >>\nstream\n".encode())
    pdf.extend(font_bytes)
    pdf.extend(b"endstream\nendobj\n")

    xref_offset = len(pdf)
    pdf.extend(b"xref\n0 8\n0000000000 65535 f \n")
    pdf.extend(f"{off1:010} 00000 n \n".encode())
    pdf.extend(f"{off2:010} 00000 n \n".encode())
    pdf.extend(f"{off3:010} 00000 n \n".encode())
    pdf.extend(f"{off4:010} 00000 n \n".encode())
    pdf.extend(f"{off5:010} 00000 n \n".encode())
    pdf.extend(f"{off6:010} 00000 n \n".encode())
    pdf.extend(f"{off7:010} 00000 n \n".encode())

    pdf.extend(b"trailer\n<< /Size 8 /Root 1 0 R >>\n")
    pdf.extend(f"startxref\n{xref_offset}\n%%EOF\n".encode())

    return bytes(pdf)


def test_font_extraction_endpoints():
    pdf_bytes = create_pdf_with_embedded_font_bytes()
    upload_resp = client.post(
        "/api/documents/upload",
        files={"file": ("font_test.pdf", io.BytesIO(pdf_bytes), "application/pdf")},
    )
    assert upload_resp.status_code == 200
    doc_id = upload_resp.json()["document_id"]

    # 1. Query page fonts list
    fonts_resp = client.get(f"/api/documents/{doc_id}/pages/1/fonts")
    assert fonts_resp.status_code == 200
    fonts_data = fonts_resp.json()
    assert fonts_data["page_number"] == 1
    assert fonts_data["embedded_count"] == 1
    assert "F1" in fonts_data["fonts"]

    # 2. Download embedded font binary
    font_bin_resp = client.get(f"/api/documents/{doc_id}/pages/1/fonts/F1")
    assert font_bin_resp.status_code == 200
    assert font_bin_resp.headers["content-type"] == "font/ttf"
    assert len(font_bin_resp.content) == 16
    assert font_bin_resp.content.startswith(b"\x00\x01\x00\x00")

    # 3. Request non-existent font returns 404
    missing_resp = client.get(f"/api/documents/{doc_id}/pages/1/fonts/NonExistentFont")
    assert missing_resp.status_code == 404


def create_pdf_with_image_bytes() -> bytes:
    """Generates a PDF containing an embedded Image XObject and placement CTM."""
    pdf = bytearray()
    pdf.extend(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n")

    off1 = len(pdf)
    pdf.extend(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n")

    off2 = len(pdf)
    pdf.extend(b"2 0 obj\n<< /Type /Pages /Kids [ 3 0 R ] /Count 1 >>\nendobj\n")

    off3 = len(pdf)
    pdf.extend(
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [ 0 0 612 792 ] /Resources << /XObject << /Im1 5 0 R >> >> /Contents 4 0 R >>\nendobj\n"
    )

    off4 = len(pdf)
    stream_content = b"q\n150 0 0 75 80 500 cm\n/Im1 Do\nQ\n"
    pdf.extend(f"4 0 obj\n<< /Length {len(stream_content)} >>\nstream\n".encode())
    pdf.extend(stream_content)
    pdf.extend(b"endstream\nendobj\n")

    off5 = len(pdf)
    raw_pixels = b"\xFF\x00\x00\x00\xFF\x00\x00\x00\xFF\xFF\xFF\xFF"  # 2x2 RGB samples
    pdf.extend(
        f"5 0 obj\n<< /Type /XObject /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length {len(raw_pixels)} >>\nstream\n".encode()
    )
    pdf.extend(raw_pixels)
    pdf.extend(b"endstream\nendobj\n")

    xref_offset = len(pdf)
    pdf.extend(b"xref\n0 6\n0000000000 65535 f \n")
    pdf.extend(f"{off1:010} 00000 n \n".encode())
    pdf.extend(f"{off2:010} 00000 n \n".encode())
    pdf.extend(f"{off3:010} 00000 n \n".encode())
    pdf.extend(f"{off4:010} 00000 n \n".encode())
    pdf.extend(f"{off5:010} 00000 n \n".encode())

    pdf.extend(b"trailer\n<< /Size 6 /Root 1 0 R >>\n")
    pdf.extend(f"startxref\n{xref_offset}\n%%EOF\n".encode())

    return bytes(pdf)


def test_image_extraction_and_replacement_endpoints():
    pdf_bytes = create_pdf_with_image_bytes()
    upload_resp = client.post(
        "/api/documents/upload",
        files={"file": ("image_test.pdf", io.BytesIO(pdf_bytes), "application/pdf")},
    )
    assert upload_resp.status_code == 200
    doc_id = upload_resp.json()["document_id"]

    # 1. Query page images and verify placement BBox
    images_resp = client.get(f"/api/documents/{doc_id}/pages/1/images")
    assert images_resp.status_code == 200
    img_data = images_resp.json()
    assert img_data["page_number"] == 1
    assert img_data["count"] == 1

    img = img_data["images"][0]
    assert img["name"] == "Im1"
    assert img["id"] == 5
    assert img["width_px"] == 2
    assert img["height_px"] == 2
    assert img["bbox"]["min_x"] == 80.0
    assert img["bbox"]["min_y"] == 500.0
    assert img["bbox"]["width"] == 150.0
    assert img["bbox"]["height"] == 75.0

    # 2. Download image binary (synthesized PNG for raw samples)
    img_bin_resp = client.get(f"/api/documents/{doc_id}/images/5")
    assert img_bin_resp.status_code == 200
    assert img_bin_resp.headers["content-type"] == "image/png"
    assert img_bin_resp.content.startswith(b"\x89PNG")

    # 3. Surgically replace image with a new JPEG image (10x20 pixels)
    # JPEG minimal header: SOI, SOF0 (height=10, width=20, components=3), EOI
    replacement_jpeg = bytearray([0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x08, 0x08, 0x00, 0x0A, 0x00, 0x14, 0x03, 0xFF, 0xD9])
    replace_resp = client.post(
        f"/api/documents/{doc_id}/images/5/replace",
        files={"file": ("new_logo.jpg", io.BytesIO(replacement_jpeg), "image/jpeg")},
    )
    assert replace_resp.status_code == 200
    assert replace_resp.json()["success"] is True

    # 4. Re-query page images and verify updated dimensions while BBox is preserved
    updated_resp = client.get(f"/api/documents/{doc_id}/pages/1/images")
    assert updated_resp.status_code == 200
    updated_img = updated_resp.json()["images"][0]
    assert updated_img["width_px"] == 20
    assert updated_img["height_px"] == 10
    assert updated_img["filter"] == "DCTDecode"
    assert updated_img["bbox"]["min_x"] == 80.0
    assert updated_img["bbox"]["min_y"] == 500.0
    assert updated_img["bbox"]["width"] == 150.0
    assert updated_img["bbox"]["height"] == 75.0

    # 5. Download replaced binary (should return image/jpeg)
    new_bin_resp = client.get(f"/api/documents/{doc_id}/images/5")
    assert new_bin_resp.status_code == 200
    assert new_bin_resp.headers["content-type"] == "image/jpeg"
    assert new_bin_resp.content == bytes(replacement_jpeg)

    # 6. Verify Export reflects the surgical change
    export_resp = client.get(f"/api/documents/{doc_id}/export")
    assert export_resp.status_code == 200
    assert b"/Filter /DCTDecode" in export_resp.content


def create_pdf_with_acroform_bytes() -> bytes:
    """Generates a valid PDF containing an interactive AcroForm with text, checkbox, and choice fields."""
    pdf = bytearray()
    pdf.extend(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n")

    off1 = len(pdf)
    pdf.extend(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R /AcroForm 5 0 R >>\nendobj\n")

    off2 = len(pdf)
    pdf.extend(b"2 0 obj\n<< /Type /Pages /Kids [ 3 0 R ] /Count 1 >>\nendobj\n")

    off3 = len(pdf)
    pdf.extend(
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [ 0 0 612 792 ] /Contents 4 0 R /Annots [ 6 0 R 7 0 R 8 0 R ] >>\nendobj\n"
    )

    off4 = len(pdf)
    stream_content = b"q\nBT\n/F1 12 Tf\n100 700 Td\n(Interactive Form Document) Tj\nET\nQ\n"
    pdf.extend(f"4 0 obj\n<< /Length {len(stream_content)} >>\nstream\n".encode())
    pdf.extend(stream_content)
    pdf.extend(b"endstream\nendobj\n")

    off5 = len(pdf)
    pdf.extend(b"5 0 obj\n<< /Fields [ 6 0 R 7 0 R 8 0 R ] /NeedAppearances true >>\nendobj\n")

    # Field 1: Text Field (FullName)
    off6 = len(pdf)
    pdf.extend(
        b"6 0 obj\n<< /Type /Annot /Subtype /Widget /FT /Tx /T (FullName) /V (Jane Doe) /Rect [ 100 600 250 620 ] /P 3 0 R >>\nendobj\n"
    )

    # Field 2: Checkbox Field (Subscribe)
    off7 = len(pdf)
    pdf.extend(
        b"7 0 obj\n<< /Type /Annot /Subtype /Widget /FT /Btn /T (Subscribe) /V /Off /AS /Off /Rect [ 100 560 120 580 ] /P 3 0 R >>\nendobj\n"
    )

    # Field 3: Choice Field (Role)
    off8 = len(pdf)
    pdf.extend(
        b"8 0 obj\n<< /Type /Annot /Subtype /Widget /FT /Ch /T (Role) /V (Developer) /Opt [ (Developer) (Designer) (Manager) ] /Rect [ 100 520 220 540 ] /P 3 0 R >>\nendobj\n"
    )

    xref_offset = len(pdf)
    pdf.extend(b"xref\n0 9\n0000000000 65535 f \n")
    pdf.extend(f"{off1:010} 00000 n \n".encode())
    pdf.extend(f"{off2:010} 00000 n \n".encode())
    pdf.extend(f"{off3:010} 00000 n \n".encode())
    pdf.extend(f"{off4:010} 00000 n \n".encode())
    pdf.extend(f"{off5:010} 00000 n \n".encode())
    pdf.extend(f"{off6:010} 00000 n \n".encode())
    pdf.extend(f"{off7:010} 00000 n \n".encode())
    pdf.extend(f"{off8:010} 00000 n \n".encode())

    pdf.extend(b"trailer\n<< /Size 9 /Root 1 0 R >>\n")
    pdf.extend(f"startxref\n{xref_offset}\n%%EOF\n".encode())

    return bytes(pdf)


def test_acroform_endpoints():
    pdf_bytes = create_pdf_with_acroform_bytes()
    upload_resp = client.post(
        "/api/documents/upload",
        files={"file": ("acroform_test.pdf", io.BytesIO(pdf_bytes), "application/pdf")},
    )
    assert upload_resp.status_code == 200
    doc_id = upload_resp.json()["document_id"]

    # 1. Query interactive form fields
    forms_resp = client.get(f"/api/documents/{doc_id}/forms")
    assert forms_resp.status_code == 200
    forms_data = forms_resp.json()
    assert forms_data["count"] == 3
    fields = {f["name"]: f for f in forms_data["fields"]}

    assert "FullName" in fields
    assert fields["FullName"]["field_type"] == "Text"
    assert fields["FullName"]["value"] == "Jane Doe"
    assert fields["FullName"]["bbox"]["min_x"] == 100.0
    assert fields["FullName"]["bbox"]["min_y"] == 600.0

    assert "Subscribe" in fields
    assert fields["Subscribe"]["field_type"] == "Checkbox"
    assert fields["Subscribe"]["value"] == "Off"

    assert "Role" in fields
    assert fields["Role"]["field_type"] == "Choice"
    assert fields["Role"]["value"] == "Developer"
    assert fields["Role"]["options"] == ["Developer", "Designer", "Manager"]

    # 2. Batch fill form fields
    fill_resp = client.post(
        f"/api/documents/{doc_id}/forms/fill",
        json={"fields": {"FullName": "Alex Mercer", "Subscribe": "Yes", "Role": "Manager"}},
    )
    assert fill_resp.status_code == 200
    assert fill_resp.json()["updated_count"] == 3

    # 3. Verify updated field values
    updated_forms_resp = client.get(f"/api/documents/{doc_id}/forms")
    assert updated_forms_resp.status_code == 200
    updated_fields = {f["name"]: f for f in updated_forms_resp.json()["fields"]}
    assert updated_fields["FullName"]["value"] == "Alex Mercer"
    assert updated_fields["Subscribe"]["value"] == "Yes"
    assert updated_fields["Role"]["value"] == "Manager"

    # 4. Flatten all form fields into permanent page graphics
    flatten_resp = client.post(f"/api/documents/{doc_id}/forms/flatten")
    assert flatten_resp.status_code == 200
    assert flatten_resp.json()["flattened_count"] == 3

    # 5. Verify forms count is now 0 (completely flattened)
    post_flatten_resp = client.get(f"/api/documents/{doc_id}/forms")
    assert post_flatten_resp.status_code == 200
    assert post_flatten_resp.json()["count"] == 0

    # 6. Verify exported binary contains the burned text
    export_resp = client.get(f"/api/documents/{doc_id}/export")
    assert export_resp.status_code == 200
    assert b"Alex Mercer" in export_resp.content
    assert b"Flattened AcroForm Fields" in export_resp.content


def test_document_operations_rotate_split_merge_reorder_delete():
    # 1. Upload Doc A (1 page) and Doc B (1 page)
    pdf_bytes = create_minimal_pdf_bytes()
    upload_resp_a = client.post(
        "/api/documents/upload",
        files={"file": ("doc_a.pdf", io.BytesIO(pdf_bytes), "application/pdf")},
    )
    assert upload_resp_a.status_code == 200
    doc_id_a = upload_resp_a.json()["document_id"]

    upload_resp_b = client.post(
        "/api/documents/upload",
        files={"file": ("doc_b.pdf", io.BytesIO(pdf_bytes), "application/pdf")},
    )
    assert upload_resp_b.status_code == 200
    doc_id_b = upload_resp_b.json()["document_id"]

    # 2. Test Merge Documents: Merge doc_a and doc_b
    merge_resp = client.post(
        "/api/documents/merge",
        json={"document_ids": [doc_id_a, doc_id_b]},
    )
    assert merge_resp.status_code == 200
    merge_data = merge_resp.json()
    merged_id = merge_data["merged_document_id"]
    assert merge_data["page_count"] == 2

    # 3. Test Rotate Page: Rotate Page 1 of merged doc by 90 degrees
    rotate_resp = client.post(
        f"/api/documents/{merged_id}/pages/1/rotate",
        json={"degrees": 90},
    )
    assert rotate_resp.status_code == 200
    assert rotate_resp.json()["new_rotation"] == 90

    # 4. Test Split Document: Extract Page 1 from merged document
    split_resp = client.post(
        f"/api/documents/{merged_id}/split",
        json={"page_indices": [0]},
    )
    assert split_resp.status_code == 200
    split_data = split_resp.json()
    assert split_data["count"] == 1
    extracted_id = split_data["extracted_document_ids"][0]

    # Verify extracted document has 1 page
    extracted_sg = client.get(f"/api/documents/{extracted_id}/pages/1/scenegraph")
    assert extracted_sg.status_code == 200

    # 5. Test Reorder Pages: Reorder merged doc [1, 0]
    reorder_resp = client.post(
        f"/api/documents/{merged_id}/pages/reorder",
        json={"new_order": [1, 0]},
    )
    assert reorder_resp.status_code == 200
    assert reorder_resp.json()["page_count"] == 2

    # 6. Test Delete Page: Delete page index 1
    delete_resp = client.post(
        f"/api/documents/{merged_id}/pages/delete",
        json={"page_indices": [1]},
    )
    assert delete_resp.status_code == 200
    assert delete_resp.json()["page_count"] == 1


def test_annotations_workflow():
    """Validates complete lifecycle of text markup, links, stamps, deletion, and flattening."""
    pdf_bytes = create_minimal_pdf_bytes()
    upload_resp = client.post(
        "/api/documents/upload",
        files={"file": ("annot_test.pdf", io.BytesIO(pdf_bytes), "application/pdf")},
    )
    assert upload_resp.status_code == 200
    doc_id = upload_resp.json()["document_id"]

    # 1. Initial annotations should be empty
    list_resp = client.get(f"/api/documents/{doc_id}/pages/1/annotations")
    assert list_resp.status_code == 200
    assert list_resp.json()["count"] == 0

    # 2. Add Highlight markup
    hl_resp = client.post(
        f"/api/documents/{doc_id}/pages/1/annotations/markup",
        json={
            "subtype": "Highlight",
            "min_x": 50.0,
            "min_y": 690.0,
            "max_x": 250.0,
            "max_y": 715.0,
            "color": [1.0, 0.9, 0.1],
            "opacity": 0.5,
            "contents": "Key Contract Clause",
        },
    )
    assert hl_resp.status_code == 200
    hl_data = hl_resp.json()
    assert hl_data["success"] is True
    hl_id = hl_data["annotation_id"]

    # 3. Add Web Link
    link_resp = client.post(
        f"/api/documents/{doc_id}/pages/1/annotations/link",
        json={
            "min_x": 50.0,
            "min_y": 650.0,
            "max_x": 200.0,
            "max_y": 670.0,
            "uri": "https://example.com/terms",
            "show_border": True,
        },
    )
    assert link_resp.status_code == 200
    link_data = link_resp.json()
    assert link_data["success"] is True

    # 4. Add Stamp
    stamp_resp = client.post(
        f"/api/documents/{doc_id}/pages/1/annotations/stamp",
        json={
            "stamp_type": "Approved",
            "min_x": 400.0,
            "min_y": 700.0,
            "max_x": 550.0,
            "max_y": 750.0,
            "date_str": "2026-10-02",
        },
    )
    assert stamp_resp.status_code == 200
    stamp_data = stamp_resp.json()
    assert stamp_data["success"] is True
    stamp_id = stamp_data["annotation_id"]

    # 5. Verify all 3 annotations are returned
    list_resp2 = client.get(f"/api/documents/{doc_id}/pages/1/annotations")
    assert list_resp2.status_code == 200
    items = list_resp2.json()["annotations"]
    assert len(items) == 3
    subtypes = [it["subtype"] for it in items]
    assert "Highlight" in subtypes
    assert "Link" in subtypes
    assert "Stamp" in subtypes

    # 6. Delete Stamp
    del_resp = client.delete(f"/api/documents/{doc_id}/pages/1/annotations/{stamp_id}")
    assert del_resp.status_code == 200
    list_resp3 = client.get(f"/api/documents/{doc_id}/pages/1/annotations")
    assert list_resp3.json()["count"] == 2

    # 7. Flatten Annotations (Highlight becomes permanent vector graphics, Link is preserved)
    flat_resp = client.post(f"/api/documents/{doc_id}/annotations/flatten?page_number=1")
    assert flat_resp.status_code == 200
    assert flat_resp.json()["flattened_count"] == 1

    # Remaining active annotation should only be the interactive Link
    list_resp4 = client.get(f"/api/documents/{doc_id}/pages/1/annotations")
    assert list_resp4.json()["count"] == 1
    assert list_resp4.json()["annotations"][0]["subtype"] == "Link"

    # 8. Export document and verify valid bytes
    export_resp = client.get(f"/api/documents/{doc_id}/export")
    assert export_resp.status_code == 200
    assert export_resp.content.startswith(b"%PDF-")


def test_pagination_and_watermarks_workflow():
    """Validates dynamic pagination and semitransparent text/image watermark REST endpoints."""
    # 1. Upload Document
    pdf_bytes = create_minimal_pdf_bytes()
    upload_resp = client.post(
        "/api/documents/upload",
        files={"file": ("watermark_doc.pdf", pdf_bytes, "application/pdf")},
    )
    assert upload_resp.status_code == 200
    doc_id = upload_resp.json()["document_id"]

    # 2. Add Dynamic Pagination
    pag_resp = client.post(
        f"/api/documents/{doc_id}/pagination",
        json={
            "format": "Página {page} de {total}",
            "position": "bottom_center",
            "font_size": 9.0,
            "color": [0.3, 0.3, 0.3],
            "margin": 36.0,
            "start_page_num": 1,
            "skip_first_page": False,
        },
    )
    assert pag_resp.status_code == 200
    pag_data = pag_resp.json()
    assert pag_data["success"] is True
    assert pag_data["affected_pages"] == 1

    # 3. Add Semi-transparent Text Watermark
    wm_text_resp = client.post(
        f"/api/documents/{doc_id}/watermark/text",
        json={
            "text": "CONFIDENCIAL",
            "font_size": 50.0,
            "color": [0.85, 0.15, 0.15],
            "opacity": 0.22,
            "rotation_degrees": 45.0,
            "placement": "background",
        },
    )
    assert wm_text_resp.status_code == 200
    wm_text_data = wm_text_resp.json()
    assert wm_text_data["success"] is True
    assert wm_text_data["affected_pages"] == 1

    # 4. Add Semi-transparent Image Watermark
    # Minimal 1x1 PNG bytes
    png_bytes = bytes([
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A,
        0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
        0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01,
        0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53,
        0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41,
        0x54, 0x08, 0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00,
        0x00, 0x03, 0x01, 0x01, 0x00, 0x18, 0xDD, 0x8D,
        0xB0, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E,
        0x44, 0xAE, 0x42, 0x60, 0x82,
    ])
    wm_img_resp = client.post(
        f"/api/documents/{doc_id}/watermark/image",
        files={"file": ("stamp.png", png_bytes, "image/png")},
        data={
            "width": "120.0",
            "height": "120.0",
            "opacity": "0.30",
            "rotation_degrees": "0.0",
            "placement": "background",
        },
    )
    assert wm_img_resp.status_code == 200
    wm_img_data = wm_img_resp.json()
    assert wm_img_data["success"] is True
    assert wm_img_data["affected_pages"] == 1

    # 5. Export and verify content
    export_resp = client.get(f"/api/documents/{doc_id}/export")
    assert export_resp.status_code == 200
    assert export_resp.content.startswith(b"%PDF-")
    assert b"CONFIDENCIAL" in export_resp.content
    assert b"P\xc3\xa1gina 1 de 1" in export_resp.content or b"de 1" in export_resp.content


def test_redaction_and_sanitization_workflow():
    """Validates irreversible content redaction and document metadata scrubbing endpoints."""
    # 1. Create a PDF with sensitive text and metadata
    pdf = bytearray()
    pdf.extend(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n")

    off1 = len(pdf)
    pdf.extend(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n")

    off2 = len(pdf)
    pdf.extend(b"2 0 obj\n<< /Type /Pages /Kids [ 3 0 R ] /Count 1 >>\nendobj\n")

    off3 = len(pdf)
    pdf.extend(
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [ 0 0 612 792 ] /Contents 4 0 R /Annots [ 5 0 R ] >>\nendobj\n"
    )

    off4 = len(pdf)
    stream_content = b"BT\n/F1 12 Tf\n1 0 0 1 72 700 Tm\n(Contact agent at classified@intel.gov for code 123-45-6789) Tj\nET\n"
    pdf.extend(f"4 0 obj\n<< /Length {len(stream_content)} >>\nstream\n".encode())
    pdf.extend(stream_content)
    pdf.extend(b"\nendstream\nendobj\n")

    off5 = len(pdf)
    pdf.extend(
        b"5 0 obj\n<< /Type /Annot /Subtype /Link /Rect [ 150 690 300 715 ] >>\nendobj\n"
    )

    off6 = len(pdf)
    pdf.extend(
        b"6 0 obj\n<< /Author (Special Agent) /Title (Secret Operation) >>\nendobj\n"
    )

    xref_offset = len(pdf)
    pdf.extend(b"xref\n0 7\n0000000000 65535 f \n")
    pdf.extend(f"{off1:010} 00000 n \n".encode())
    pdf.extend(f"{off2:010} 00000 n \n".encode())
    pdf.extend(f"{off3:010} 00000 n \n".encode())
    pdf.extend(f"{off4:010} 00000 n \n".encode())
    pdf.extend(f"{off5:010} 00000 n \n".encode())
    pdf.extend(f"{off6:010} 00000 n \n".encode())

    pdf.extend(b"trailer\n<< /Size 7 /Root 1 0 R /Info 6 0 R >>\n")
    pdf.extend(f"startxref\n{xref_offset}\n%%EOF\n".encode())
    pdf_bytes = bytes(pdf)

    # 2. Upload document
    upload_resp = client.post(
        "/api/documents/upload",
        files={"file": ("classified.pdf", io.BytesIO(pdf_bytes), "application/pdf")},
    )
    assert upload_resp.status_code == 200
    doc_id = upload_resp.json()["document_id"]

    # 3. Test Pattern Redaction (Email) with metadata scrubbing and annotation pruning
    pattern_resp = client.post(
        f"/api/documents/{doc_id}/redact/pattern",
        json={
            "pattern_type": "email",
            "overlay_text": "[CENSURADO]",
            "scrub_metadata": True,
            "prune_annotations": True,
        },
    )
    assert pattern_resp.status_code == 200
    pattern_data = pattern_resp.json()
    assert pattern_data["success"] is True
    assert pattern_data["total_blackout_boxes"] == 1
    assert pattern_data["total_purged_glyphs"] > 0
    assert pattern_data["total_pruned_annotations"] == 1

    # Verify content stream: email MUST BE PURGED
    export_resp1 = client.get(f"/api/documents/{doc_id}/export")
    assert export_resp1.status_code == 200
    assert b"classified@intel.gov" not in export_resp1.content
    assert b"Contact agent at" in export_resp1.content
    assert b"[CENSURADO]" in export_resp1.content

    # 4. Test Text Redaction: redact "123-45-6789"
    text_resp = client.post(
        f"/api/documents/{doc_id}/redact/text",
        json={
            "query": "123-45-6789",
            "overlay_text": "[TOP SECRET]",
        },
    )
    assert text_resp.status_code == 200
    text_data = text_resp.json()
    assert text_data["success"] is True
    assert text_data["total_blackout_boxes"] == 1

    # Verify content stream: SSN MUST BE PURGED
    export_resp2 = client.get(f"/api/documents/{doc_id}/export")
    assert export_resp2.status_code == 200
    assert b"123-45-6789" not in export_resp2.content
    assert b"[TOP SECRET]" in export_resp2.content

    # 5. Test Regions Redaction: redact coordinates [70, 695, 120, 715] ("Contact")
    regions_resp = client.post(
        f"/api/documents/{doc_id}/redact/regions",
        json={
            "page_number": 1,
            "regions": [
                {"min_x": 70.0, "min_y": 695.0, "max_x": 120.0, "max_y": 715.0}
            ],
            "overlay_text": "",
        },
    )
    assert regions_resp.status_code == 200
    assert regions_resp.json()["success"] is True

    # 6. Test Document Sanitization
    sanitize_resp = client.post(
        f"/api/documents/{doc_id}/sanitize",
        json={"scrub_metadata": True},
    )
    assert sanitize_resp.status_code == 200
    assert sanitize_resp.json()["success"] is True


def test_security_and_digital_signatures_workflow():
    """Validates encryption, permissions, decryption, and digital signatures API workflow."""
    pdf_bytes = create_minimal_pdf_bytes()
    upload_resp = client.post(
        "/api/documents/upload",
        files={"file": ("contract.pdf", pdf_bytes, "application/pdf")},
    )
    assert upload_resp.status_code == 200
    doc_id = upload_resp.json()["document_id"]

    # 1. Initial security status: not encrypted, 0 signatures
    sec_resp = client.get(f"/api/documents/{doc_id}/security")
    assert sec_resp.status_code == 200
    sec_data = sec_resp.json()
    assert sec_data["is_encrypted"] is False
    assert len(sec_data["signatures"]) == 0

    # 2. Inscribe digital signature
    sign_resp = client.post(
        f"/api/documents/{doc_id}/security/sign",
        json={
            "signer_name": "Lic. Roberto Garduño",
            "reason": "Dictamen Legal Aprobatorio",
            "location": "Ciudad de México, MX",
            "page_number": 1,
            "rect": [72.0, 72.0, 272.0, 142.0],
            "contact_info": "rgarduno@pdfengine.com",
        },
    )
    assert sign_resp.status_code == 200
    sign_data = sign_resp.json()
    assert sign_data["success"] is True
    assert sign_data["signature"]["signer_name"] == "Lic. Roberto Garduño"
    assert sign_data["signature"]["sub_filter"] == "PDFEngine.sha256"
    assert sign_data["signature"]["byte_range"] != [0, 1024, 2048, 4096]
    assert len(sign_data["signature"]["contents_hex"]) == 64
    assert sign_data["signature"]["byte_range_valid"] is True

    signed_export = client.get(f"/api/documents/{doc_id}/export")
    assert signed_export.status_code == 200
    assert b"/PDFEngine.sha256" in signed_export.content
    assert b"/PDFEngine.Approval" in signed_export.content
    assert b"adbe.pkcs7.detached" not in signed_export.content
    assert b"Adobe.PPKLite" not in signed_export.content

    reupload = client.post(
        "/api/documents/upload",
        files={"file": ("signed.pdf", signed_export.content, "application/pdf")},
    )
    assert reupload.status_code == 200
    signed_id = reupload.json()["document_id"]
    reread = client.get(f"/api/documents/{signed_id}/security/signatures")
    assert reread.status_code == 200
    reread_sigs = reread.json()
    assert len(reread_sigs) == 1
    assert reread_sigs[0]["byte_range_valid"] is True
    assert reread_sigs[0]["sub_filter"] == "PDFEngine.sha256"

    # 3. Verify signatures list endpoint
    sigs_resp = client.get(f"/api/documents/{doc_id}/security/signatures")
    assert sigs_resp.status_code == 200
    sigs_list = sigs_resp.json()
    assert len(sigs_list) == 1
    assert sigs_list[0]["signer_name"] == "Lic. Roberto Garduño"
    assert sigs_list[0]["location"] == "Ciudad de México, MX"

    # 4. Encrypt document with AES-128 and granular permissions
    encrypt_resp = client.post(
        f"/api/documents/{doc_id}/security/encrypt",
        json={
            "user_password": "secret_user",
            "owner_password": "secret_admin",
            "permissions": {
                "print_low_res": True,
                "print_high_res": False,
                "modify_contents": False,
                "copy_extract": False,
                "modify_annotations": False,
                "fill_forms": False,
                "accessibility_extract": True,
                "assemble_document": False,
            },
            "encrypt_metadata": True,
        },
    )
    assert encrypt_resp.status_code == 200
    enc_data = encrypt_resp.json()
    assert enc_data["success"] is True
    assert enc_data["is_encrypted"] is True

    # Check status endpoint reflects encryption
    sec_resp2 = client.get(f"/api/documents/{doc_id}/security")
    assert sec_resp2.status_code == 200
    assert sec_resp2.json()["is_encrypted"] is True

    # Verify exported PDF contains /Encrypt
    export_enc = client.get(f"/api/documents/{doc_id}/export")
    assert export_enc.status_code == 200
    assert b"/Encrypt" in export_enc.content

    # 5. Decrypt document
    decrypt_resp = client.post(
        f"/api/documents/{doc_id}/security/decrypt",
        json={"password": "secret_user"},
    )
    assert decrypt_resp.status_code == 200
    dec_data = decrypt_resp.json()
    assert dec_data["success"] is True
    assert dec_data["is_encrypted"] is False

    # Check status endpoint reflects decryption
    sec_resp3 = client.get(f"/api/documents/{doc_id}/security")
    assert sec_resp3.status_code == 200
    assert sec_resp3.json()["is_encrypted"] is False


def test_encrypt_requires_owner_password_and_hides_plaintext():
    pdf_bytes = create_minimal_pdf_bytes()
    upload = client.post(
        "/api/documents/upload",
        files={"file": ("contract.pdf", pdf_bytes, "application/pdf")},
    )
    assert upload.status_code == 200
    doc_id = upload.json()["document_id"]

    missing = client.post(
        f"/api/documents/{doc_id}/security/encrypt",
        json={"user_password": "secret_user"},
    )
    assert missing.status_code == 422
    assert "owner_password" in missing.text

    empty = client.post(
        f"/api/documents/{doc_id}/security/encrypt",
        json={"user_password": "secret_user", "owner_password": ""},
    )
    assert empty.status_code == 422

    def encrypt_fresh() -> bytes:
        uploaded = client.post(
            "/api/documents/upload",
            files={"file": ("contract.pdf", pdf_bytes, "application/pdf")},
        )
        assert uploaded.status_code == 200
        new_id = uploaded.json()["document_id"]
        response = client.post(
            f"/api/documents/{new_id}/security/encrypt",
            json={
                "user_password": "secret_user",
                "owner_password": "secret_admin",
            },
        )
        assert response.status_code == 200
        exported = client.get(f"/api/documents/{new_id}/export")
        assert exported.status_code == 200
        assert b"/Encrypt" in exported.content
        assert b"Contract Agreement Terms" not in exported.content
        return exported.content

    first = encrypt_fresh()
    second = encrypt_fresh()
    assert first != second

    reupload = client.post(
        "/api/documents/upload",
        files={"file": ("encrypted.pdf", first, "application/pdf")},
    )
    assert reupload.status_code == 200
    encrypted_id = reupload.json()["document_id"]
    decrypted = client.post(
        f"/api/documents/{encrypted_id}/security/decrypt",
        json={"password": "secret_admin"},
    )
    assert decrypted.status_code == 200
    restored = client.get(f"/api/documents/{encrypted_id}/export")
    assert restored.status_code == 200
    assert b"Contract Agreement Terms" in restored.content
    assert b"/Encrypt" not in restored.content


def create_pdf_with_table_bytes() -> bytes:
    """Creates a valid PDF containing a 2x2 table with vector lines and cell text."""
    pdf = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")

    off1 = len(pdf)
    pdf.extend(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n")

    off2 = len(pdf)
    pdf.extend(b"2 0 obj\n<< /Type /Pages /Kids [ 3 0 R ] /Count 1 >>\nendobj\n")

    off3 = len(pdf)
    pdf.extend(
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [ 0 0 612 792 ] /Contents 4 0 R >>\nendobj\n"
    )

    off4 = len(pdf)
    stream_content = (
        # Horizontal lines (700, 650, 600 from 100 to 300)
        b"100 700 m 300 700 l S\n"
        b"100 650 m 300 650 l S\n"
        b"100 600 m 300 600 l S\n"
        # Vertical lines (100, 200, 300 from 600 to 700)
        b"100 600 m 100 700 l S\n"
        b"200 600 m 200 700 l S\n"
        b"300 600 m 300 700 l S\n"
        # Text in cell (0, 0)
        b"BT\n/F1 10 Tf\n110 670 Tm\n(Item) Tj\nET\n"
        # Text in cell (0, 1)
        b"BT\n/F1 10 Tf\n210 670 Tm\n(Price) Tj\nET\n"
        # Text in cell (1, 0)
        b"BT\n/F1 10 Tf\n110 620 Tm\n(Widget) Tj\nET\n"
        # Text in cell (1, 1)
        b"BT\n/F1 10 Tf\n210 620 Tm\n($100) Tj\nET\n"
    )
    pdf.extend(f"4 0 obj\n<< /Length {len(stream_content)} >>\nstream\n".encode())
    pdf.extend(stream_content)
    pdf.extend(b"endstream\nendobj\n")

    xref_offset = len(pdf)
    pdf.extend(b"xref\n0 5\n0000000000 65535 f \n")
    pdf.extend(f"{off1:010} 00000 n \n".encode())
    pdf.extend(f"{off2:010} 00000 n \n".encode())
    pdf.extend(f"{off3:010} 00000 n \n".encode())
    pdf.extend(f"{off4:010} 00000 n \n".encode())

    pdf.extend(b"trailer\n<< /Size 5 /Root 1 0 R >>\n")
    pdf.extend(f"startxref\n{xref_offset}\n%%EOF\n".encode())

    return bytes(pdf)


def test_table_detection_and_export_workflow():
    pdf_bytes = create_pdf_with_table_bytes()

    # 1. Upload Document with Table
    upload_resp = client.post(
        "/api/documents/upload",
        files={"file": ("invoice_table.pdf", io.BytesIO(pdf_bytes), "application/pdf")},
    )
    assert upload_resp.status_code == 200
    doc_id = upload_resp.json()["document_id"]

    # 2. Extract tables on page 1
    tables_resp = client.get(f"/api/documents/{doc_id}/pages/1/tables")
    assert tables_resp.status_code == 200
    data = tables_resp.json()
    assert data["total_tables"] >= 1

    table = data["tables"][0]
    assert table["row_count"] == 2
    assert table["col_count"] == 2
    assert table["headers"] == ["Item", "Price"]
    assert table["rows"] == [["Widget", "$100"]]
    assert len(table["cells"]) == 4

    # 3. Export table as CSV
    csv_resp = client.get(f"/api/documents/{doc_id}/pages/1/tables/0/export?format=csv")
    assert csv_resp.status_code == 200
    csv_data = csv_resp.json()
    assert "Item,Price" in csv_data["content"]
    assert "Widget,$100" in csv_data["content"]

    # 4. Export table as Markdown
    md_resp = client.get(f"/api/documents/{doc_id}/pages/1/tables/0/export?format=markdown")
    assert md_resp.status_code == 200
    md_data = md_resp.json()
    assert "| Item | Price |" in md_data["content"]
    assert "| Widget | $100 |" in md_data["content"]

    # 5. Export table as JSON
    json_resp = client.get(f"/api/documents/{doc_id}/pages/1/tables/0/export?format=json")
    assert json_resp.status_code == 200
    json_data = json_resp.json()
    assert "\"headers\": [\"Item\", \"Price\"]" in json_data["content"]

    # 6. Test CSV file download
    dl_resp = client.get(f"/api/documents/{doc_id}/pages/1/tables/0/export?format=csv&download=true")
    assert dl_resp.status_code == 200
    assert "attachment; filename=" in dl_resp.headers.get("Content-Disposition", "")
    assert b"Item,Price" in dl_resp.content

    # 7. HTML export stays a JSON string, and the file download is plain text.
    html_resp = client.get(f"/api/documents/{doc_id}/pages/1/tables/0/export?format=html")
    assert html_resp.status_code == 200
    assert html_resp.headers["content-type"].startswith("application/json")
    assert "<table" in html_resp.json()["content"]
    assert "<script" not in html_resp.json()["content"].lower()

    html_dl = client.get(
        f"/api/documents/{doc_id}/pages/1/tables/0/export?format=html&download=true"
    )
    assert html_dl.status_code == 200
    assert html_dl.headers["content-type"].startswith("text/plain")
    assert html_dl.headers["x-content-type-options"] == "nosniff"
    assert html_dl.headers["content-disposition"] == 'attachment; filename="table_p1_0.html"'
    assert b"<table" in html_dl.content
    assert b"<script" not in html_dl.content.lower()


def test_document_pages_overview_and_rotation_endpoints():
    pdf_bytes = create_minimal_pdf_bytes()

    # 1. Upload Doc
    upload_resp = client.post(
        "/api/documents/upload",
        files={"file": ("overview_test.pdf", io.BytesIO(pdf_bytes), "application/pdf")},
    )
    assert upload_resp.status_code == 200
    doc_id = upload_resp.json()["document_id"]

    # 2. Get pages overview
    overview_resp = client.get(f"/api/documents/{doc_id}/pages/overview")
    assert overview_resp.status_code == 200
    data = overview_resp.json()
    assert data["document_id"] == doc_id
    assert data["total_pages"] == 1
    assert len(data["pages"]) == 1
    assert data["pages"][0]["page_number"] == 1
    assert data["pages"][0]["rotation"] == 0
    assert data["pages"][0]["paragraph_count"] >= 1

    # 3. Rotate page and verify overview reflects new rotation
    rot_resp = client.post(f"/api/documents/{doc_id}/pages/1/rotate", json={"degrees": 90})
    assert rot_resp.status_code == 200
    assert rot_resp.json()["new_rotation"] == 90

    # 4. Check single page rotation endpoint
    page_rot_resp = client.get(f"/api/documents/{doc_id}/pages/1/rotation")
    assert page_rot_resp.status_code == 200
    assert page_rot_resp.json()["rotation"] == 90

    # 5. Check updated overview reflects 90 degrees
    updated_overview_resp = client.get(f"/api/documents/{doc_id}/pages/overview")
    assert updated_overview_resp.status_code == 200
    updated = updated_overview_resp.json()
    assert updated["pages"][0]["rotation"] == 90
    assert updated["offset"] == 0
    assert updated["limit"] == 24

    # 6. A request names its own window. Past the last page the list is empty,
    # and a limit above the per-request ceiling is clamped.
    window_resp = client.get(
        f"/api/documents/{doc_id}/pages/overview",
        params={"offset": 0, "limit": 1},
    )
    assert window_resp.status_code == 200
    window = window_resp.json()
    assert window["limit"] == 1
    assert len(window["pages"]) == 1
    assert window["total_pages"] == 1

    past_end = client.get(
        f"/api/documents/{doc_id}/pages/overview",
        params={"offset": 1, "limit": 24},
    )
    assert past_end.status_code == 200
    assert past_end.json()["pages"] == []
    assert past_end.json()["total_pages"] == 1
    assert past_end.json()["offset"] == 1

    clamped = client.get(
        f"/api/documents/{doc_id}/pages/overview",
        params={"limit": 1000},
    )
    assert clamped.status_code == 200
    assert clamped.json()["limit"] == 48
    assert len(clamped.json()["pages"]) == 1


def test_document_optimization_and_export_endpoints():
    pdf_bytes = create_minimal_pdf_bytes()

    # 1. Upload Document
    upload_resp = client.post(
        "/api/documents/upload",
        files={"file": ("optimize_sample.pdf", io.BytesIO(pdf_bytes), "application/pdf")},
    )
    assert upload_resp.status_code == 200
    doc_id = upload_resp.json()["document_id"]

    # 2. Call Optimize Endpoint
    opt_resp = client.post(
        f"/api/documents/{doc_id}/optimize",
        json={
            "remove_unused": True,
            "pack_object_streams": True,
            "recompress_flate": True,
            "deduplicate_streams": True,
            "max_objects_per_stream": 50,
        },
    )
    assert opt_resp.status_code == 200
    data = opt_resp.json()
    assert data["success"] is True
    assert data["document_id"] == doc_id
    assert data["original_size"] > 0
    assert data["optimized_size"] > 0
    assert "Optimización completada con éxito" in data["message"]

    # 3. Export Optimized PDF
    export_resp = client.get(f"/api/documents/{doc_id}/export?optimized=true")
    assert export_resp.status_code == 200
    assert export_resp.headers["content-type"] == "application/pdf"
    assert (
        export_resp.headers["content-disposition"]
        == 'attachment; filename="document_optimized.pdf"'
    )
    assert len(export_resp.content) == data["optimized_size"]

    # 4. Verify Document Session is active and paragraphs still readable
    para_resp = client.get(f"/api/documents/{doc_id}/pages/1/scenegraph")
    assert para_resp.status_code == 200
    para_data = para_resp.json()
    assert len(para_data["paragraphs"]) >= 1
    assert "Contract Agreement Terms" in para_data["paragraphs"][0]["text"]


def test_optimize_refuses_signed_and_encrypted_documents():
    pdf_bytes = create_minimal_pdf_bytes()
    upload = client.post(
        "/api/documents/upload",
        files={"file": ("protected.pdf", pdf_bytes, "application/pdf")},
    )
    assert upload.status_code == 200
    doc_id = upload.json()["document_id"]

    signed = client.post(
        f"/api/documents/{doc_id}/security/sign",
        json={
            "signer_name": "Ana Ruiz",
            "reason": "Archivo",
            "location": "CDMX",
            "page_number": 1,
            "rect": [72.0, 72.0, 272.0, 142.0],
        },
    )
    assert signed.status_code == 200

    refused = client.post(f"/api/documents/{doc_id}/optimize", json={})
    assert refused.status_code == 409
    assert (
        refused.json()["detail"]
        == "Optimization is refused for encrypted or signed documents."
    )

    export_opt = client.get(f"/api/documents/{doc_id}/export?optimized=true")
    assert export_opt.status_code == 409
    assert (
        export_opt.json()["detail"]
        == "Optimization is refused for encrypted or signed documents."
    )

    still = client.get(f"/api/documents/{doc_id}/security/signatures")
    assert still.status_code == 200
    assert still.json()[0]["byte_range_valid"] is True

    plain = client.get(f"/api/documents/{doc_id}/export")
    assert plain.status_code == 200
    assert b"/PDFEngine.sha256" in plain.content

    encrypted_upload = client.post(
        "/api/documents/upload",
        files={"file": ("secret.pdf", pdf_bytes, "application/pdf")},
    )
    assert encrypted_upload.status_code == 200
    enc_id = encrypted_upload.json()["document_id"]
    encrypted = client.post(
        f"/api/documents/{enc_id}/security/encrypt",
        json={"user_password": "secret_user", "owner_password": "secret_admin"},
    )
    assert encrypted.status_code == 200

    refused_enc = client.post(f"/api/documents/{enc_id}/optimize", json={})
    assert refused_enc.status_code == 409
    assert (
        refused_enc.json()["detail"]
        == "Optimization is refused for encrypted or signed documents."
    )
    status = client.get(f"/api/documents/{enc_id}/security")
    assert status.status_code == 200
    assert status.json()["is_encrypted"] is True
    exported = client.get(f"/api/documents/{enc_id}/export")
    assert exported.status_code == 200
    assert b"/Encrypt" in exported.content


def test_form_builder_crud_workflow():
    """Validates creation, updating, retrieval, deletion, and flattening of form fields."""
    pdf_bytes = create_minimal_pdf_bytes()
    upload_res = client.post(
        "/api/documents/upload",
        files={"file": ("form_builder_test.pdf", io.BytesIO(pdf_bytes), "application/pdf")},
    )
    assert upload_res.status_code == 200
    doc_id = upload_res.json()["document_id"]

    # 1. Create a Text field
    create_text_resp = client.post(
        f"/api/documents/{doc_id}/pages/1/forms",
        json={
            "name": "CustomerEmail",
            "field_type": "Text",
            "min_x": 72.0,
            "min_y": 700.0,
            "max_x": 300.0,
            "max_y": 724.0,
            "value": "dev@company.com",
            "alt_name": "Customer Email Address",
            "is_required": True,
            "font_size": 11.0,
        },
    )
    assert create_text_resp.status_code == 200
    text_data = create_text_resp.json()
    assert text_data["status"] == "ok"
    assert text_data["field"]["name"] == "CustomerEmail"
    assert text_data["field"]["field_type"] == "Text"
    assert text_data["field"]["value"] == "dev@company.com"
    assert text_data["field"]["is_required"] is True

    # 2. Create a Checkbox field
    create_chk_resp = client.post(
        f"/api/documents/{doc_id}/pages/1/forms",
        json={
            "name": "AcceptClause",
            "field_type": "Checkbox",
            "min_x": 72.0,
            "min_y": 660.0,
            "max_x": 92.0,
            "max_y": 680.0,
            "value": "Yes",
            "alt_name": "Accept conditions",
        },
    )
    assert create_chk_resp.status_code == 200
    chk_data = create_chk_resp.json()
    assert chk_data["field"]["name"] == "AcceptClause"
    assert chk_data["field"]["field_type"] == "Checkbox"

    # 3. Create a Choice dropdown field
    create_choice_resp = client.post(
        f"/api/documents/{doc_id}/pages/1/forms",
        json={
            "name": "ServiceTier",
            "field_type": "Choice",
            "min_x": 72.0,
            "min_y": 620.0,
            "max_x": 220.0,
            "max_y": 644.0,
            "value": "Gold",
            "options": ["Silver", "Gold", "Platinum"],
        },
    )
    assert create_choice_resp.status_code == 200
    choice_data = create_choice_resp.json()
    assert choice_data["field"]["name"] == "ServiceTier"
    assert choice_data["field"]["field_type"] == "Choice"
    assert len(choice_data["field"]["options"]) == 3

    # 4. Create a Signature field
    create_sig_resp = client.post(
        f"/api/documents/{doc_id}/pages/1/forms",
        json={
            "name": "LegalSign",
            "field_type": "Signature",
            "min_x": 72.0,
            "min_y": 550.0,
            "max_x": 272.0,
            "max_y": 600.0,
            "alt_name": "Sign here",
        },
    )
    assert create_sig_resp.status_code == 200
    assert create_sig_resp.json()["field"]["field_type"] == "Signature"

    # 5. List all forms
    list_resp = client.get(f"/api/documents/{doc_id}/forms")
    assert list_resp.status_code == 200
    assert list_resp.json()["count"] == 4

    # 6. Update text field properties
    update_resp = client.put(
        f"/api/documents/{doc_id}/forms/CustomerEmail",
        json={
            "min_x": 80.0,
            "min_y": 705.0,
            "max_x": 320.0,
            "max_y": 730.0,
            "alt_name": "Corporate Email Updated",
            "is_read_only": True,
        },
    )
    assert update_resp.status_code == 200
    upd_data = update_resp.json()
    assert upd_data["field"]["bbox"]["min_x"] == 80.0
    assert upd_data["field"]["alt_name"] == "Corporate Email Updated"
    assert upd_data["field"]["is_read_only"] is True

    # 7. Delete one field
    del_resp = client.delete(f"/api/documents/{doc_id}/forms/AcceptClause")
    assert del_resp.status_code == 200
    assert del_resp.json()["deleted"] is True

    # 8. Verify list count is now 3
    list_resp2 = client.get(f"/api/documents/{doc_id}/forms")
    assert list_resp2.status_code == 200
    assert list_resp2.json()["count"] == 3
    names = [f["name"] for f in list_resp2.json()["fields"]]
    assert "AcceptClause" not in names
    assert "CustomerEmail" in names
    assert "ServiceTier" in names
    assert "LegalSign" in names

    # 9. Duplicate field name returns 400
    dup_resp = client.post(
        f"/api/documents/{doc_id}/pages/1/forms",
        json={"name": "CustomerEmail", "field_type": "Text"},
    )
    assert dup_resp.status_code == 400


def test_health_does_not_require_a_token():
    response = anonymous.get("/api/health")
    assert response.status_code == 200
    assert response.json()["status"] == "healthy"


def test_upload_without_token_is_rejected():
    response = anonymous.post(
        "/api/documents/upload",
        files={"file": ("agreement.pdf", create_minimal_pdf_bytes(), "application/pdf")},
    )
    assert response.status_code == 401
    assert response.json()["detail"] == "Authentication required."


def test_foreign_subject_cannot_open_a_document():
    upload_resp = client.post(
        "/api/documents/upload",
        files={"file": ("owned.pdf", create_minimal_pdf_bytes(), "application/pdf")},
    )
    assert upload_resp.status_code == 200
    doc_id = upload_resp.json()["document_id"]

    denied = other_client.get(f"/api/documents/{doc_id}/pages/1/scenegraph")
    assert denied.status_code == 404
    assert denied.json()["detail"] == "Document session not found."

    owned = client.get(f"/api/documents/{doc_id}/pages/1/scenegraph")
    assert owned.status_code == 200


def test_ws_ticket_is_single_use_and_requires_a_bearer():
    refused = anonymous.post("/api/auth/ws-ticket")
    assert refused.status_code == 401

    issued = client.post("/api/auth/ws-ticket")
    assert issued.status_code == 200
    body = issued.json()
    assert body["expires_in"] == 60
    assert isinstance(body["ticket"], str) and len(body["ticket"]) >= 20

    upload_resp = client.post(
        "/api/documents/upload",
        files={"file": ("ticketed.pdf", create_minimal_pdf_bytes(), "application/pdf")},
    )
    doc_id = upload_resp.json()["document_id"]
    ticket = body["ticket"]

    with client.websocket_connect(
        f"/ws/documents/{doc_id}/pages/1/reflow?ticket={ticket}"
    ) as websocket:
        websocket.close()

    rejected = False
    try:
        with client.websocket_connect(
            f"/ws/documents/{doc_id}/pages/1/reflow?ticket={ticket}"
        ) as websocket:
            websocket.receive_text()
    except WebSocketDisconnect as exc:
        rejected = True
        assert exc.code == 1008
    assert rejected


def test_unlisted_and_wildcard_origins_are_not_reflected(monkeypatch):
    monkeypatch.delenv("PDFENGINE_CORS_ORIGINS", raising=False)
    unset = anonymous.get("/api/health", headers={"Origin": "http://evil.example"})
    assert unset.status_code == 200
    assert "access-control-allow-origin" not in unset.headers
    assert "access-control-allow-credentials" not in unset.headers

    monkeypatch.setenv("PDFENGINE_CORS_ORIGINS", "*")
    wildcard = anonymous.get("/api/health", headers={"Origin": "http://evil.example"})
    assert wildcard.status_code == 200
    assert "access-control-allow-origin" not in wildcard.headers
    assert "access-control-allow-credentials" not in wildcard.headers


def test_listed_origin_is_reflected_with_credentials(monkeypatch):
    monkeypatch.setenv("PDFENGINE_CORS_ORIGINS", "http://localhost:3000")
    allowed = anonymous.get("/api/health", headers={"Origin": "http://localhost:3000"})
    assert allowed.status_code == 200
    assert allowed.headers["access-control-allow-origin"] == "http://localhost:3000"
    assert allowed.headers["access-control-allow-credentials"] == "true"

    denied = anonymous.get("/api/health", headers={"Origin": "http://evil.example"})
    assert denied.status_code == 200
    assert "access-control-allow-origin" not in denied.headers
    assert "access-control-allow-credentials" not in denied.headers

    unauthorized = anonymous.get(
        "/api/documents/missing/pages/1/scenegraph",
        headers={"Origin": "http://localhost:3000"},
    )
    assert unauthorized.status_code == 401
    assert unauthorized.headers["access-control-allow-origin"] == "http://localhost:3000"
    assert unauthorized.headers["access-control-allow-credentials"] == "true"


def test_preflight_allows_only_a_listed_origin(monkeypatch):
    monkeypatch.setenv("PDFENGINE_CORS_ORIGINS", "http://localhost:3000")
    allowed = anonymous.options(
        "/api/health",
        headers={
            "Origin": "http://localhost:3000",
            "Access-Control-Request-Method": "GET",
            "Access-Control-Request-Headers": "authorization,content-type",
        },
    )
    assert allowed.status_code == 204
    assert allowed.headers["access-control-allow-origin"] == "http://localhost:3000"
    assert allowed.headers["access-control-allow-credentials"] == "true"
    assert allowed.headers["access-control-allow-headers"] == "authorization,content-type"
    assert allowed.text == ""

    denied = anonymous.options(
        "/api/health",
        headers={
            "Origin": "http://evil.example",
            "Access-Control-Request-Method": "GET",
        },
    )
    assert denied.status_code == 400
    assert "access-control-allow-origin" not in denied.headers
    assert "access-control-allow-credentials" not in denied.headers
    assert "evil.example" not in denied.text


def test_parse_failure_hides_engine_text():
    response = client.post(
        "/api/documents/upload",
        files={"file": ("broken.pdf", b"%PDF-1.7\n", "application/pdf")},
    )
    assert response.status_code == 422
    detail = response.json()["detail"]
    assert detail == "The request could not be completed."
    assert "startxref" not in response.text
    assert "offset" not in response.text


def test_export_ignores_the_uploaded_filename():
    pdf_bytes = create_minimal_pdf_bytes()
    upload_resp = client.post(
        "/api/documents/upload",
        files={
            "file": (
                'evil"; filename="pwned.pdf',
                io.BytesIO(pdf_bytes),
                "application/pdf",
            )
        },
    )
    assert upload_resp.status_code == 200
    stored_name = upload_resp.json()["filename"]
    assert "pwned" in stored_name
    assert stored_name != "document_edited.pdf"
    doc_id = upload_resp.json()["document_id"]

    export_resp = client.get(f"/api/documents/{doc_id}/export")
    assert export_resp.status_code == 200
    assert (
        export_resp.headers["content-disposition"]
        == 'attachment; filename="document_edited.pdf"'
    )
    assert stored_name not in export_resp.headers["content-disposition"]
    assert "pwned" not in export_resp.headers["content-disposition"]
    assert "\r" not in export_resp.headers["content-disposition"]
    assert "\n" not in export_resp.headers["content-disposition"]


def test_link_endpoint_accepts_only_web_schemes():
    pdf_bytes = create_minimal_pdf_bytes()
    upload_resp = client.post(
        "/api/documents/upload",
        files={"file": ("links.pdf", io.BytesIO(pdf_bytes), "application/pdf")},
    )
    assert upload_resp.status_code == 200
    doc_id = upload_resp.json()["document_id"]
    box = {
        "min_x": 50.0,
        "min_y": 650.0,
        "max_x": 200.0,
        "max_y": 670.0,
        "show_border": False,
    }
    for uri in ("javascript:example", "file:example"):
        refused = client.post(
            f"/api/documents/{doc_id}/pages/1/annotations/link",
            json={**box, "uri": uri},
        )
        assert refused.status_code == 400
        assert refused.json()["detail"] == "Only http and https links are accepted."

    listed = client.get(f"/api/documents/{doc_id}/pages/1/annotations")
    assert listed.status_code == 200
    assert listed.json()["count"] == 0

    accepted = client.post(
        f"/api/documents/{doc_id}/pages/1/annotations/link",
        json={**box, "uri": "HTTPS://example.com/terms#javascript:example"},
    )
    assert accepted.status_code == 200
    listed_ok = client.get(f"/api/documents/{doc_id}/pages/1/annotations")
    assert listed_ok.json()["count"] == 1
    assert (
        listed_ok.json()["annotations"][0]["link_uri"]
        == "HTTPS://example.com/terms#javascript:example"
    )


def test_audit_log_records_upload_redact_optimize_and_sign_without_secrets():
    """The action log names who did the work and omits file contents and credentials."""
    rejected = client.post(
        "/api/documents/upload",
        files={"file": ("notes.pdf", b"not-a-pdf", "application/pdf")},
    )
    assert rejected.status_code == 400
    assert client.get("/api/audit").json() == []

    pdf_bytes = create_minimal_pdf_bytes()
    upload = client.post(
        "/api/documents/upload",
        files={"file": ("contract.pdf", io.BytesIO(pdf_bytes), "application/pdf")},
    )
    assert upload.status_code == 200
    doc_id = upload.json()["document_id"]

    redacted = client.post(
        f"/api/documents/{doc_id}/redact/text",
        json={"query": "Contract", "overlay_text": "hidden-query"},
    )
    assert redacted.status_code == 200

    optimized = client.post(f"/api/documents/{doc_id}/optimize", json={})
    assert optimized.status_code == 200

    signed = client.post(
        f"/api/documents/{doc_id}/security/sign",
        json={
            "signer_name": "Ana Ruiz",
            "reason": "Archivo",
            "location": "CDMX",
            "page_number": 1,
            "contact_info": "ana@example.com",
        },
    )
    assert signed.status_code == 200

    refused = client.post(f"/api/documents/{doc_id}/optimize", json={})
    assert refused.status_code == 409

    audit = client.get("/api/audit")
    assert audit.status_code == 200
    rows = audit.json()
    assert [row["action"] for row in rows] == ["upload", "redact", "optimize", "sign"]
    for row in rows:
        assert set(row) == {"action", "document_id", "at"}
        assert row["document_id"] == doc_id
        assert len(row["at"]) == 20 and row["at"].endswith("Z")
    body = audit.text
    assert "Contract" not in body
    assert "hidden-query" not in body
    assert "contract.pdf" not in body
    assert "pdfengine-test-key" not in body
    assert "ana@example.com" not in body
    assert pdf_bytes[:8].decode("latin-1") not in body

    assert other_client.get("/api/audit").json() == []
    assert anonymous.get("/api/audit").status_code == 401


def test_audit_log_drops_the_oldest_event_and_rejects_free_text():
    """A full buffer forgets the oldest row and never stores an arbitrary note."""
    assert record("upload\npassword=secret", "a" * 32, "doc-1") is False
    assert record("upload", "subject with spaces", "doc-1") is False
    assert record("upload", "a" * 32, "../doc") is False
    assert events_for("a" * 32) == []

    for index in range(MAX_AUDIT_EVENTS + 2):
        assert record("upload", "b" * 32, f"doc-{index}") is True
    kept = events_for("b" * 32)
    assert len(kept) == MAX_AUDIT_EVENTS
    assert kept[0].document_id == "doc-2"
    assert kept[-1].document_id == f"doc-{MAX_AUDIT_EVENTS + 1}"


def _subject():
    subject = authenticate_header("Bearer pdfengine-test-key-0001")
    assert subject is not None
    return subject


def test_same_document_mutations_wait_and_reads_do_not():
    """A second edit of one document waits. A read of that document does not."""
    _, upload = _upload_minimal()
    assert upload.status_code == 200
    doc_id = upload.json()["document_id"]
    subject = _subject()
    holding = threading.Event()
    release = threading.Event()
    finished = threading.Event()
    outcome: dict = {}

    def holder():
        token = adopt_subject(subject)
        try:
            with document_mutation(doc_id):
                holding.set()
                assert release.wait(timeout=3)
        finally:
            reset_subject(token)

    def contender():
        token = adopt_subject(subject)
        try:
            outcome["response"] = rotate_page_endpoint(
                doc_id, 1, RotatePageRequest(degrees=90)
            )
        except Exception as exc:
            outcome["error"] = exc
        finally:
            finished.set()
            reset_subject(token)

    first = threading.Thread(target=holder)
    second = threading.Thread(target=contender)
    first.start()
    assert holding.wait(timeout=2)
    second.start()
    assert finished.wait(timeout=0.3) is False

    token = adopt_subject(subject)
    try:
        overview = get_document_overview(doc_id)
    finally:
        reset_subject(token)
    assert overview.document_id == doc_id
    assert overview.total_pages == 1
    assert finished.is_set() is False

    release.set()
    assert finished.wait(timeout=3)
    first.join(timeout=2)
    second.join(timeout=2)
    assert first.is_alive() is False
    assert second.is_alive() is False
    assert "error" not in outcome
    assert outcome["response"].new_rotation == 90


def test_distinct_documents_mutate_at_the_same_time():
    """Holding one document does not block a mutation of another."""
    _, first_upload = _upload_minimal("a.pdf")
    _, second_upload = _upload_minimal("b.pdf")
    id_a = first_upload.json()["document_id"]
    id_b = second_upload.json()["document_id"]
    subject = _subject()
    holding = threading.Event()
    release = threading.Event()
    entered_b = threading.Event()

    def hold_a():
        token = adopt_subject(subject)
        try:
            with document_mutation(id_a):
                holding.set()
                assert release.wait(timeout=3)
        finally:
            reset_subject(token)

    def enter_b():
        token = adopt_subject(subject)
        try:
            with document_mutation(id_b):
                entered_b.set()
        finally:
            reset_subject(token)

    holder = threading.Thread(target=hold_a)
    other = threading.Thread(target=enter_b)
    holder.start()
    assert holding.wait(timeout=2)
    other.start()
    assert entered_b.wait(timeout=1)
    release.set()
    holder.join(timeout=2)
    other.join(timeout=2)
    assert holder.is_alive() is False
    assert other.is_alive() is False


def test_merging_locks_documents_in_id_order():
    """Opposite merge orders take the same lock sequence and both finish."""
    _, first_upload = _upload_minimal("a.pdf")
    _, second_upload = _upload_minimal("b.pdf")
    id_a = first_upload.json()["document_id"]
    id_b = second_upload.json()["document_id"]
    subject = _subject()
    inside = threading.Event()
    release = threading.Event()
    names: dict = {}

    def first_order():
        token = adopt_subject(subject)
        try:
            with document_mutations([id_b, id_a]) as sessions:
                names["first"] = [session["filename"] for session in sessions]
                inside.set()
                assert release.wait(timeout=3)
        finally:
            reset_subject(token)

    def second_order():
        token = adopt_subject(subject)
        try:
            with document_mutations([id_a, id_b]) as sessions:
                names["second"] = [session["filename"] for session in sessions]
        finally:
            reset_subject(token)

    earlier = threading.Thread(target=first_order)
    later = threading.Thread(target=second_order)
    earlier.start()
    assert inside.wait(timeout=2)
    later.start()
    later.join(timeout=0.3)
    assert later.is_alive()
    release.set()
    later.join(timeout=2)
    earlier.join(timeout=2)
    assert earlier.is_alive() is False
    assert later.is_alive() is False
    assert names["first"] == ["b.pdf", "a.pdf"]
    assert names["second"] == ["a.pdf", "b.pdf"]









