from pathlib import Path

from fastapi import HTTPException, UploadFile


async def extract_text(filename: str, upload: UploadFile) -> str:
    suffix = Path(filename).suffix.lower()
    content = await upload.read()

    if suffix in {".txt", ".md", ".markdown", ".text", ".csv"}:
        return content.decode("utf-8", errors="replace")
    if suffix == ".pdf":
        return _extract_pdf(content)
    if suffix in {".html", ".htm"}:
        return _extract_html(content)
    raise HTTPException(status_code=400, detail=f"Unsupported file type: {suffix or 'none'}")


def _extract_pdf(content: bytes) -> str:
    from io import BytesIO

    from pypdf import PdfReader

    reader = PdfReader(BytesIO(content))
    pages = [page.extract_text() or "" for page in reader.pages]
    return "\n\n".join(pages)


def _extract_html(content: bytes) -> str:
    import re

    text = content.decode("utf-8", errors="replace")
    text = re.sub(r"<script[^>]*>.*?</script>", " ", text, flags=re.IGNORECASE | re.DOTALL)
    text = re.sub(r"<style[^>]*>.*?</style>", " ", text, flags=re.IGNORECASE | re.DOTALL)
    text = re.sub(r"<[^>]+>", " ", text)
    text = re.sub(r"\s+", " ", text)
    return text.strip()
