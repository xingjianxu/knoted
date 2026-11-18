-- Chat message keys use the source registry's two-word type discriminator.
ALTER TABLE fulltext_publications
    DROP CONSTRAINT fulltext_publications_document_key_check,
    ADD CONSTRAINT fulltext_publications_document_key_check
        CHECK (document_key ~ '^([a-z]+|chat_message)_[0-9a-f]{32}_v[0-9]+$');
