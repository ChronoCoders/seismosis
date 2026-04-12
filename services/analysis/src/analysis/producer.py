"""
Confluent Kafka producer for enriched events, alerts, and dead-letter messages.

Enriched and alert messages use idempotent delivery (acks=all, retries=3).
Dead-letter messages use best-effort delivery (acks=1, no retries) because
DLQ failures are non-fatal — the offset is committed regardless.
"""
from __future__ import annotations

import json
import logging
from typing import Optional

from confluent_kafka import KafkaError, KafkaException, Message, Producer

log = logging.getLogger(__name__)

_BASE_CONF: dict = {
    "acks": "all",
    "enable.idempotence": True,
    "compression.type": "lz4",
    "message.timeout.ms": 30_000,
    "retries": 3,
    "retry.backoff.ms": 500,
    "linger.ms": 5,
}

_DLQ_OVERLAY: dict = {
    "acks": "1",
    "enable.idempotence": False,
    "message.timeout.ms": 5_000,
    "retries": 0,
}


class KafkaProducer:
    """Synchronous (flush-after-produce) Kafka producer for the analysis service."""

    def __init__(self, brokers: str) -> None:
        base = {**_BASE_CONF, "bootstrap.servers": brokers}
        self._producer = Producer(base)
        dlq_conf = {**base, **_DLQ_OVERLAY}
        self._dlq_producer = Producer(dlq_conf)

    def produce_enriched(self, topic: str, key: str, value: bytes) -> None:
        """
        Produce an Avro-encoded enriched event and block until acknowledged.

        Raises KafkaException after retries are exhausted.
        """
        self._produce_sync(self._producer, topic, key, value)

    def produce_alert(self, topic: str, key: str, value: bytes) -> None:
        """Produce an alert event with the same durability as enriched events."""
        self._produce_sync(self._producer, topic, key, value)

    def produce_dead_letter(
        self,
        topic: str,
        key: str,
        failure_reason: str,
        failure_detail: str,
        original_topic: str,
        partition: int,
        offset: int,
        source_id: Optional[str],
        raw_payload_hex: str,
    ) -> None:
        """
        Route a failed message to the dead-letter topic.

        Best-effort delivery (acks=1, no retry).  DLQ failures are logged at
        ERROR but are non-fatal — the Kafka offset is still committed.
        """
        envelope = json.dumps(
            {
                "failure_reason": failure_reason,
                "failure_detail": failure_detail,
                "original_topic": original_topic,
                "kafka_partition": partition,
                "kafka_offset": offset,
                "source_id": source_id,
                "raw_payload_hex": raw_payload_hex,
            }
        ).encode()
        try:
            self._produce_sync(self._dlq_producer, topic, key, envelope, timeout=5.0)
        except Exception as exc:
            log.error(
                "DLQ delivery failed (non-fatal): reason=%s source_id=%s error=%s",
                failure_reason,
                source_id,
                exc,
            )

    def flush(self, timeout: float = 30.0) -> None:
        self._producer.flush(timeout=timeout)

    def close(self) -> None:
        self._producer.flush(timeout=30)
        self._dlq_producer.flush(timeout=5)

    # ── Internal helper ────────────────────────────────────────────────────

    @staticmethod
    def _produce_sync(
        producer: Producer,
        topic: str,
        key: str,
        value: bytes,
        timeout: float = 30.0,
    ) -> None:
        error_holder: list[Optional[Exception]] = [None]

        def _on_delivery(err: Optional[KafkaError], _msg: Message) -> None:
            if err:
                error_holder[0] = KafkaException(err)

        producer.produce(
            topic, key=key.encode(), value=value, on_delivery=_on_delivery
        )
        producer.flush(timeout=timeout)
        if error_holder[0] is not None:
            raise error_holder[0]
