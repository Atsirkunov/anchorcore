from fastapi import APIRouter, Depends, HTTPException, UploadFile
from sqlalchemy import select
from sqlalchemy.orm import Session

from .. import schemas
from ..classifier import Classifier
from ..config import settings
from ..db import get_db
from ..models import Document, KnowledgeObject
from ..text_extractor import extract_text

router = APIRouter(prefix="/documents", tags=["documents"])


@router.post("", response_model=schemas.DocumentDetail, status_code=201)
async def upload_document(
    file: UploadFile,
    db: Session = Depends(get_db),
    classifier: Classifier = Depends(lambda: Classifier()),
) -> schemas.DocumentDetail:
    settings.upload_dir.mkdir(parents=True, exist_ok=True)

    text = await extract_text(file.filename or "upload", file)

    if not text.strip():
        raise HTTPException(status_code=400, detail="No text extracted from file")

    document = Document(
        filename=file.filename or "upload",
        content_type=file.content_type or "",
        text=text,
    )
    db.add(document)
    db.commit()
    db.refresh(document)

    items = await classifier.classify(text, document.filename)
    for item in items:
        db.add(KnowledgeObject(document_id=document.id, **item))
    db.commit()
    db.refresh(document)

    return document


@router.get("", response_model=list[schemas.DocumentOut])
def list_documents(db: Session = Depends(get_db)) -> list[schemas.DocumentOut]:
    documents = db.execute(select(Document).order_by(Document.created_at.desc())).scalars().all()
    return [
        schemas.DocumentOut(
            id=d.id,
            filename=d.filename,
            content_type=d.content_type,
            text_length=len(d.text),
            created_at=d.created_at,
        )
        for d in documents
    ]


@router.get("/{document_id}", response_model=schemas.DocumentDetail)
def get_document(document_id: int, db: Session = Depends(get_db)) -> schemas.DocumentDetail:
    document = db.get(Document, document_id)
    if document is None:
        raise HTTPException(status_code=404, detail="Document not found")
    return document


@router.get("/{document_id}/objects", response_model=list[schemas.KnowledgeObjectOut])
def list_objects(document_id: int, db: Session = Depends(get_db)) -> list[schemas.KnowledgeObjectOut]:
    if db.get(Document, document_id) is None:
        raise HTTPException(status_code=404, detail="Document not found")
    objects = db.execute(
        select(KnowledgeObject)
        .where(KnowledgeObject.document_id == document_id)
        .order_by(KnowledgeObject.confidence.desc())
    ).scalars().all()
    return list(objects)
