from .base import BaseConnector, ConnectorError, IngestionDoc
from .folder import FolderConnector
from .jira import JiraConnector

CONNECTORS: dict[str, type[BaseConnector]] = {
    "folder": FolderConnector,
    "jira": JiraConnector,
}


def build_connector(connector_type: str, config: dict) -> BaseConnector:
    cls = CONNECTORS.get(connector_type)
    if cls is None:
        raise ConnectorError(f"unknown connector: {connector_type}")
    return cls(config)


__all__ = ["BaseConnector", "ConnectorError", "IngestionDoc", "build_connector"]
