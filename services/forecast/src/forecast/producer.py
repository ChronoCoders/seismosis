"""
Kafka producer for the forecast service.

Publishes ETAS forecast records to the ``earthquakes.forecasts`` topic using
Avro serialisation via the Confluent Schema Registry.

Delivery is fire-and-forget (non-blocking ``produce()``); ``flush()`` is called
at the end of each main-loop cycle and at shutdown.  If ``confluent_kafka`` is
not installed the class degrades gracefully to a no-op so the service can start
in environments where Kafka is unavailable.
"""

from __future__ import annotations

import json
import logging
from typing import Any, Optional

import structlog

log: structlog.stdlib.BoundLogger = structlog.get_logger(__name__)

FORECASTS_TOPIC = "earthquakes.forecasts"

_ETAS_FORECAST_SCHEMA: dict[str, Any] = {
    "type": "record",
    "name": "EtasForecast",
    "namespace": "io.seismosis.forecasts",
    "fields": [
        {"name": "mainshock_source_id", "type": "string"},
        {"name": "computed_at", "type": "string"},
        {"name": "horizon_days", "type": "int"},
        {"name": "min_magnitude", "type": "float"},
        {"name": "expected_count", "type": "double"},
        {"name": "p_at_least_one", "type": "double"},
        {"name": "p_exceedance", "type": "string"},
        {"name": "daily_rates", "type": "string"},
        {"name": "model_version", "type": "string"},
    ],
}

_BASE_CONF: dict[str, object] = {
    "acks": "all",
    "enable.idempotence": True,
    "compression.type": "zstd",
    "message.timeout.ms": 30_000,
    "retries": 3,
    "retry.backoff.ms": 500,
    "linger.ms": 10,
}


class ForecastProducer:
    """Non-blocking Avro Kafka producer for ETAS forecast records.

    All public methods are safe to call even when ``confluent_kafka`` is not
    installed — in that case every method is a no-op and a warning is logged
    once at construction time.
    """

    def __init__(self, kafka_brokers: str, schema_registry_url: str) -> None:
        self._enabled = False
        self._producer: Any = None
        self._serializer: Any = None

        try:
            from confluent_kafka import Producer  # type: ignore[import-untyped]
            from confluent_kafka.schema_registry import (  # type: ignore[import-untyped]
                SchemaRegistryClient,
            )
            from confluent_kafka.schema_registry.avro import (  # type: ignore[import-untyped]
                AvroSerializer,
            )
            from confluent_kafka.serialization import (  # type: ignore[import-untyped]
                SerializationContext,
                MessageField,
            )

            schema_str = json.dumps(_ETAS_FORECAST_SCHEMA)
            registry_client = SchemaRegistryClient({"url": schema_registry_url})
            self._serializer = AvroSerializer(
                registry_client,
                schema_str,
            )
            self._serialization_context = SerializationContext(
                FORECASTS_TOPIC, MessageField.VALUE
            )

            conf: dict[str, object] = {
                **_BASE_CONF,
                "bootstrap.servers": kafka_brokers,
            }
            self._producer = Producer(conf)
            self._enabled = True
            log.info(
                "forecast_producer.initialized",
                brokers=kafka_brokers,
                schema_registry=schema_registry_url,
                topic=FORECASTS_TOPIC,
            )

        except ImportError:
            log.warning(
                "forecast_producer.disabled",
                reason="confluent_kafka not installed — all publish calls will be no-ops",
            )
        except Exception as exc:
            log.warning(
                "forecast_producer.init_failed",
                error=str(exc),
                reason="producer disabled for this session",
            )

    def publish_etas_forecast(
        self,
        mainshock_source_id: str,
        forecast_data: dict[str, Any],
    ) -> None:
        """Fire-and-forget publish of an ETAS forecast record.

        Does not block; call ``flush()`` to drain the internal queue.
        """
        if not self._enabled or self._producer is None:
            return

        try:
            from confluent_kafka import KafkaError, KafkaException, Message  # type: ignore[import-untyped]

            def _on_delivery(
                err: Optional[KafkaError], _msg: Message
            ) -> None:
                if err:
                    log.error(
                        "forecast_producer.delivery_error",
                        source_id=mainshock_source_id,
                        error=str(err),
                    )
                else:
                    log.debug(
                        "forecast_producer.delivered",
                        source_id=mainshock_source_id,
                    )

            value_bytes: bytes = self._serializer(
                forecast_data, self._serialization_context
            )
            self._producer.produce(
                topic=FORECASTS_TOPIC,
                key=mainshock_source_id.encode(),
                value=value_bytes,
                on_delivery=_on_delivery,
            )
            # Poll to trigger delivery callbacks without blocking
            self._producer.poll(0)

        except Exception as exc:
            log.error(
                "forecast_producer.produce_error",
                source_id=mainshock_source_id,
                error=str(exc),
            )

    def flush(self, timeout_secs: float = 5.0) -> None:
        """Flush all buffered messages, waiting up to *timeout_secs* seconds."""
        if not self._enabled or self._producer is None:
            return
        try:
            remaining: int = self._producer.flush(timeout=timeout_secs)
            if remaining > 0:
                log.warning(
                    "forecast_producer.flush_incomplete",
                    remaining=remaining,
                    timeout_secs=timeout_secs,
                )
        except Exception as exc:
            log.error("forecast_producer.flush_error", error=str(exc))

    def close(self) -> None:
        """Flush with a generous timeout and release resources."""
        if not self._enabled or self._producer is None:
            return
        try:
            self._producer.flush(timeout=30.0)
            log.info("forecast_producer.closed")
        except Exception as exc:
            log.error("forecast_producer.close_error", error=str(exc))
