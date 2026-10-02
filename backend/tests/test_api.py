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





