"""Integration tests for the PDFEngine REST API."""

import io
from fastapi.testclient import TestClient
from app.main import app

client = TestClient(app)


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


def test_health_endpoint():
    response = client.get("/api/health")
    assert response.status_code == 200
    data = response.json()
    assert data["status"] == "healthy"
    assert data["version"] == "0.1.0"


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
    assert "agreement_edited.pdf" in export_resp.headers["content-disposition"]
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

    # Connect to WebSocket reflow channel
    with client.websocket_connect(f"/ws/documents/{doc_id}/pages/1/reflow") as websocket:
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

