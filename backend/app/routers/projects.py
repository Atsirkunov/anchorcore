"""Projects / scoped search (B15): named bundles of sources.

GET/POST/PATCH/DELETE over projects, plus setting the default scope. A source
can belong to multiple projects; queries scoped to a project only retrieve
chunks from that project's sources.
"""

from fastapi import APIRouter, Depends, HTTPException
from sqlalchemy import select, update
from sqlalchemy.orm import Session

from .. import schemas
from ..db import get_db
from ..models import Project, Source


def make_router() -> APIRouter:
    router = APIRouter(prefix="/projects", tags=["projects"])

    @router.get("", response_model=list[schemas.ProjectOut])
    def list_projects(db: Session = Depends(get_db)) -> list[dict]:
        projects = db.execute(select(Project).order_by(Project.created_at)).scalars().all()
        out = []
        for p in projects:
            d = _project_dict(p)
            out.append(d)
        return out

    @router.post("", response_model=schemas.ProjectOut, status_code=201)
    def create_project(payload: schemas.ProjectCreate, db: Session = Depends(get_db)) -> dict:
        if not payload.name.strip():
            raise HTTPException(status_code=422, detail="project name is required")
        # validate sources BEFORE any write — leaves the DB untouched on 422
        sources = _sources_for(db, payload.source_ids) if payload.source_ids else []
        project = Project(name=payload.name.strip())
        db.add(project)
        db.flush()
        project.sources = sources
        db.commit()
        db.refresh(project)
        return _project_dict(project, db)

    @router.get("/default", response_model=schemas.ProjectOut | None)
    def default_project(db: Session = Depends(get_db)) -> dict | None:
        project = db.execute(
            select(Project).where(Project.is_default.is_(True)).limit(1)
        ).scalar_one_or_none()
        if project is None:
            return None
        return _project_dict(project, db)

    @router.get("/{project_id}", response_model=schemas.ProjectOut)
    def get_project(project_id: int, db: Session = Depends(get_db)) -> dict:
        project = db.get(Project, project_id)
        if project is None:
            raise HTTPException(status_code=404, detail="Project not found")
        return _project_dict(project, db)

    @router.patch("/{project_id}", response_model=schemas.ProjectOut)
    def update_project(
        project_id: int, payload: schemas.ProjectUpdate, db: Session = Depends(get_db)
    ) -> dict:
        project = db.get(Project, project_id)
        if project is None:
            raise HTTPException(status_code=404, detail="Project not found")
        if payload.name is not None:
            if not payload.name.strip():
                raise HTTPException(status_code=422, detail="project name cannot be empty")
            project.name = payload.name.strip()
        if payload.is_default is not None and payload.is_default:
            _clear_default(db, except_id=project.id)
            project.is_default = True
        if payload.source_ids is not None:
            project.sources = _sources_for(db, payload.source_ids)
        db.commit()
        db.refresh(project)
        return _project_dict(project, db)

    @router.delete("/{project_id}")
    def delete_project(project_id: int, db: Session = Depends(get_db)) -> dict:
        project = db.get(Project, project_id)
        if project is None:
            raise HTTPException(status_code=404, detail="Project not found")
        db.delete(project)
        db.commit()
        return {"deleted": True}

    return router


def _clear_default(db: Session, except_id: int | None = None) -> None:
    stmt = update(Project).where(Project.is_default.is_(True))
    if except_id is not None:
        stmt = stmt.where(Project.id != except_id)
    db.execute(stmt.values(is_default=False))


def _sources_for(db: Session, source_ids: list[int]) -> list[Source]:
    sources = db.execute(select(Source).where(Source.id.in_(source_ids))).scalars().all()
    if len(sources) != len(set(source_ids)):
        found = {s.id for s in sources}
        missing = [sid for sid in source_ids if sid not in found]
        raise HTTPException(status_code=422, detail=f"unknown source ids: {missing}")
    return sources


def _project_dict(project: Project, db: Session | None = None) -> dict:
    """ProjectOut supports both a loaded session (for source_ids) and the
    already-loaded relationship (list endpoint)."""
    if db is not None:
        source_ids = [s.id for s in db.execute(
            select(Source).join(Project.sources).where(Project.id == project.id).order_by(Source.id)
        ).scalars()]
    else:
        source_ids = sorted(s.id for s in project.sources)
    return {
        "id": project.id,
        "name": project.name,
        "is_default": project.is_default,
        "created_at": project.created_at,
        "source_ids": source_ids,
    }
