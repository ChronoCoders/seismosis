"""
Avro encode/decode with Confluent Schema Registry wire format.

Wire format (Confluent):
    0x00          — magic byte (1 byte)
    schema_id     — big-endian uint32 (4 bytes)
    avro_binary   — fastavro schemaless binary (remainder)

The SchemaRegistry client caches parsed schemas by ID to avoid repeated
HTTP round-trips.
"""
from __future__ import annotations

import io
import json
import struct
from typing import Any

import fastavro
import requests

_MAGIC_BYTE = 0x00
_HEADER_LEN = 5  # 1 magic + 4 schema_id


class SchemaRegistryError(Exception):
    """Raised when the Schema Registry returns an unexpected response."""


class SchemaRegistry:
    """Minimal HTTP client for a Confluent-compatible Schema Registry."""

    def __init__(self, url: str) -> None:
        self._url = url.rstrip("/")
        self._session = requests.Session()
        self._session.headers.update(
            {"Content-Type": "application/vnd.schemaregistry.v1+json"}
        )
        self._by_id: dict[int, Any] = {}  # schema_id → parsed fastavro schema

    def register(self, subject: str, schema: dict) -> int:
        """
        Register *schema* under *subject* and return its integer ID.

        Idempotent: re-registering an identical schema returns the same ID.

        Raises SchemaRegistryError on non-2xx responses.
        """
        resp = self._session.post(
            f"{self._url}/subjects/{subject}/versions",
            json={"schema": json.dumps(schema)},
            timeout=10,
        )
        if resp.status_code not in (200, 201):
            raise SchemaRegistryError(
                f"register({subject!r}): HTTP {resp.status_code} — {resp.text}"
            )
        return int(resp.json()["id"])

    def get_parsed(self, schema_id: int) -> Any:
        """
        Return the cached parsed fastavro schema for *schema_id*.

        Fetches from the registry on the first call, then serves from cache.
        """
        if schema_id in self._by_id:
            return self._by_id[schema_id]
        resp = self._session.get(
            f"{self._url}/schemas/ids/{schema_id}", timeout=10
        )
        if resp.status_code != 200:
            raise SchemaRegistryError(
                f"get_parsed({schema_id}): HTTP {resp.status_code} — {resp.text}"
            )
        raw_dict = json.loads(resp.json()["schema"])
        parsed = fastavro.parse_schema(raw_dict)
        self._by_id[schema_id] = parsed
        return parsed


class AvroDecoder:
    """Decode Confluent wire-format Avro messages from a Kafka topic."""

    def __init__(self, registry: SchemaRegistry) -> None:
        self._registry = registry

    def decode(self, data: bytes) -> dict[str, Any]:
        """
        Decode *data* from Confluent wire format to a Python dict.

        Raises ValueError if the magic byte is missing or the message is
        shorter than the 5-byte header.
        """
        if len(data) < _HEADER_LEN:
            raise ValueError(
                f"Message too short for Confluent wire format: {len(data)} bytes"
            )
        if data[0] != _MAGIC_BYTE:
            raise ValueError(
                f"Expected magic byte 0x00, got 0x{data[0]:02x}"
            )
        schema_id = struct.unpack(">I", data[1:5])[0]
        schema = self._registry.get_parsed(schema_id)
        return fastavro.schemaless_reader(io.BytesIO(data[5:]), schema)


class AvroEncoder:
    """Encode Python dicts to Confluent wire-format Avro bytes."""

    def __init__(self, schema: Any, schema_id: int) -> None:
        self._schema = schema
        # Pre-build the 5-byte header once.
        self._header = bytes([_MAGIC_BYTE]) + struct.pack(">I", schema_id)

    def encode(self, record: dict[str, Any]) -> bytes:
        """Encode *record* and prepend the Confluent wire-format header."""
        bio = io.BytesIO()
        fastavro.schemaless_writer(bio, self._schema, record)
        return self._header + bio.getvalue()
