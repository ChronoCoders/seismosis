"""Confluent Kafka consumer wrapper."""
from __future__ import annotations

from typing import Optional

from confluent_kafka import Consumer, KafkaError, KafkaException, Message


class KafkaConsumer:
    """
    Thin wrapper around confluent_kafka.Consumer.

    Handles subscription and normalises poll() return values so callers
    only see None (no message / soft error) or a valid Message.
    Hard errors raise KafkaException.
    """

    def __init__(self, brokers: str, group_id: str, topic: str) -> None:
        self._consumer = Consumer(
            {
                "bootstrap.servers": brokers,
                "group.id": group_id,
                "auto.offset.reset": "earliest",
                "enable.auto.commit": False,
                # Processing budget: decode + enrich + produce + DB + cache.
                # Well within 5 minutes even for worst-case DB/Redis latency.
                "max.poll.interval.ms": 300_000,
                "session.timeout.ms": 30_000,
                "heartbeat.interval.ms": 10_000,
                "log.connection.close": False,
            }
        )
        self._consumer.subscribe([topic])

    def poll(self, timeout: float = 1.0) -> Optional[Message]:
        """
        Poll for one message.

        Returns None on timeout or partition EOF.
        Raises KafkaException on unrecoverable errors.
        """
        msg: Optional[Message] = self._consumer.poll(timeout=timeout)
        if msg is None:
            return None
        if msg.error():
            err: KafkaError = msg.error()
            if err.code() == KafkaError._PARTITION_EOF:
                return None
            raise KafkaException(err)
        return msg

    def commit(self, message: Message) -> None:
        """Synchronously commit the offset past *message*."""
        self._consumer.commit(message=message, asynchronous=False)

    def close(self) -> None:
        """Commit pending offsets and close the consumer."""
        self._consumer.close()
