-- Stable typed identity survives business deletion; never use a path as an FK.
ALTER TABLE entity_registry ADD CONSTRAINT entity_registry_id_type_unique UNIQUE(id,entity_type);
CREATE TABLE tags (
 id uuid PRIMARY KEY, name text NOT NULL CHECK(char_length(name) BETWEEN 1 AND 64),
 version bigint NOT NULL DEFAULT 1 CHECK(version>0),
 created_at timestamptz NOT NULL, updated_at timestamptz NOT NULL, deleted_at timestamptz
);
CREATE UNIQUE INDEX tags_live_name ON tags(name) WHERE deleted_at IS NULL;
CREATE TABLE note_tags (
 entity_id uuid NOT NULL REFERENCES notes(id), entity_type text NOT NULL DEFAULT 'note' CHECK(entity_type='note'),
 tag_id uuid NOT NULL REFERENCES tags(id), PRIMARY KEY(entity_id,tag_id),
 FOREIGN KEY(entity_id,entity_type) REFERENCES entity_registry(id,entity_type)
);
CREATE TABLE file_tags (
 entity_id uuid NOT NULL, entity_type text NOT NULL DEFAULT 'file' CHECK(entity_type='file'),
 tag_id uuid NOT NULL REFERENCES tags(id), PRIMARY KEY(entity_id,tag_id),
 FOREIGN KEY(entity_id,entity_type) REFERENCES entity_registry(id,entity_type)
);
CREATE TABLE project_tags (
 entity_id uuid NOT NULL, entity_type text NOT NULL DEFAULT 'project' CHECK(entity_type='project'),
 tag_id uuid NOT NULL REFERENCES tags(id), PRIMARY KEY(entity_id,tag_id),
 FOREIGN KEY(entity_id,entity_type) REFERENCES entity_registry(id,entity_type)
);
CREATE TABLE task_tags (
 entity_id uuid NOT NULL, entity_type text NOT NULL DEFAULT 'task' CHECK(entity_type='task'),
 tag_id uuid NOT NULL REFERENCES tags(id), PRIMARY KEY(entity_id,tag_id),
 FOREIGN KEY(entity_id,entity_type) REFERENCES entity_registry(id,entity_type)
);
CREATE TABLE entity_links (
 id uuid PRIMARY KEY,
 source_type text NOT NULL CHECK(source_type IN ('note','file','project','task')), source_id uuid NOT NULL,
 target_type text NOT NULL CHECK(target_type IN ('note','file','project','task')), target_id uuid NOT NULL,
 version bigint NOT NULL DEFAULT 1 CHECK(version>0), created_at timestamptz NOT NULL,
 FOREIGN KEY(source_id,source_type) REFERENCES entity_registry(id,entity_type),
 FOREIGN KEY(target_id,target_type) REFERENCES entity_registry(id,entity_type),
 UNIQUE(source_id,target_id)
);
CREATE INDEX entity_links_source_order ON entity_links(source_id,id);
