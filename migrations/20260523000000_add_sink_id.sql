CREATE TABLE uploaded_invoices_new (
    source_id TEXT NOT NULL,
    sink_id TEXT NOT NULL,
    external_id TEXT NOT NULL,
    sink_reference TEXT,
    invoice_issued_at TEXT NOT NULL,
    uploaded_at TEXT NOT NULL,
    PRIMARY KEY (source_id, sink_id, external_id)
);

INSERT INTO uploaded_invoices_new
    (source_id, sink_id, external_id, sink_reference, invoice_issued_at, uploaded_at)
SELECT
    source_id,
    'paperless',
    external_id,
    paperless_task_id,
    invoice_issued_at,
    uploaded_at
FROM uploaded_invoices;

DROP TABLE uploaded_invoices;

ALTER TABLE uploaded_invoices_new RENAME TO uploaded_invoices;

CREATE INDEX idx_uploaded_invoices_source_date
    ON uploaded_invoices (source_id, invoice_issued_at);
