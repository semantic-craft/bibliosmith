#!/usr/bin/env node
// Shared, presentation-independent compiler for standard and bilingual EPUB builds.
const fs = require('fs');
const path = require('path');

const INTERNAL_TITLE = /^(?:(?:chapter|unit|section)[-_ ]*\d+|continuation(?:\s+\d+)?)$/i;
const CANONICAL_ID = /^[A-Za-z][A-Za-z0-9_-]*$/;
const ROLES = new Set(['frontmatter', 'bodymatter', 'backmatter']);

function requireCanonicalId(value, label) {
  if (typeof value !== 'string' || !CANONICAL_ID.test(value)) {
    throw new Error(`${label} must be a canonical ID matching ${CANONICAL_ID}.`);
  }
  return value;
}

function readJson(file) {
  return JSON.parse(fs.readFileSync(file, 'utf8'));
}

function compilePublicationStructure(projectRoot) {
  const root = path.resolve(projectRoot);
  const publicationMap = readJson(path.join(root, 'metadata', 'publication_map.json'));
  const sourceMap = readJson(path.join(root, 'metadata', 'source_map.json'));
  const sourceMarkdown = fs.readFileSync(path.join(root, 'source', 'source.md'), 'utf8');
  const sourceLineCount = sourceMarkdown.length === 0
    ? 0
    : sourceMarkdown.split(/\r?\n/).length - (sourceMarkdown.endsWith('\n') ? 1 : 0);
  if (publicationMap.schema !== 'local-reading-publication-map-v1'
      || !publicationMap.audit || publicationMap.audit.status !== 'passed') {
    throw new Error('A passed local-reading-publication-map-v1 is required before build_reading.');
  }
  if (sourceMap.schema !== 'local-reading-source-map-v2') {
    throw new Error('local-reading-source-map-v2 is required before build_reading.');
  }
  if (!Array.isArray(publicationMap.sections) || !publicationMap.sections.length
      || !Array.isArray(sourceMap.translationUnits) || !sourceMap.translationUnits.length) {
    throw new Error('Publication sections and translation units must both be non-empty.');
  }

  const sections = publicationMap.sections.map((section) => ({ ...section }));
  const byId = new Map();
  const ordinals = new Set();
  for (const section of sections) {
    requireCanonicalId(section.id, 'Publication section ID');
    if (section.parentId !== null && section.parentId !== undefined) {
      requireCanonicalId(section.parentId, `Parent ID for ${section.id}`);
    }
    const sourceTitle = typeof section.title === 'string' ? section.title.trim() : '';
    const title = typeof section.readerTitle === 'string' ? section.readerTitle.trim() : '';
    if (!section.id || !sourceTitle || !title || INTERNAL_TITLE.test(title)) {
      throw new Error(`Invalid reader-visible publication title: ${section.id || 'missing-id'}`);
    }
    if (byId.has(section.id)) throw new Error(`Duplicate publication section ID: ${section.id}`);
    if (!Number.isInteger(section.ordinal) || section.ordinal < 1
        || section.ordinal > sections.length || ordinals.has(section.ordinal)) {
      throw new Error(`Publication section has an invalid ordinal: ${section.id}`);
    }
    ordinals.add(section.ordinal);
    if (!Number.isInteger(section.headingLevel)
        || section.headingLevel < 1 || section.headingLevel > 6) {
      throw new Error(`Publication section has an invalid heading level: ${section.id}`);
    }
    if (!Number.isInteger(section.sourceStartLine)
        || !Number.isInteger(section.sourceEndLine)
        || section.sourceStartLine < 1
        || section.sourceEndLine < section.sourceStartLine
        || section.sourceEndLine > sourceLineCount) {
      throw new Error(`Publication section has an invalid source range: ${section.id}`);
    }
    if (!ROLES.has(section.role)) {
      throw new Error(`Publication section has an invalid role: ${section.id}`);
    }
    section.sourceTitle = sourceTitle;
    section.title = title;
    section.shortTitle = typeof section.readerShortTitle === 'string'
      && section.readerShortTitle.trim() ? section.readerShortTitle.trim() : title;
    byId.set(section.id, section);
  }

  const rootById = new Map();
  const depthById = new Map();
  for (const section of sections) {
    const visited = new Set([section.id]);
    let current = section;
    let depth = 1;
    while (current.parentId) {
      const parent = byId.get(current.parentId);
      if (!parent) throw new Error(`Publication section has a missing parent: ${section.id}`);
      if (visited.has(parent.id)) throw new Error(`Publication hierarchy contains a cycle: ${section.id}`);
      if (current.headingLevel <= parent.headingLevel
          || current.headingLevel > parent.headingLevel + 1) {
        throw new Error(`Publication section has an invalid parent depth: ${current.id}`);
      }
      if (current.sourceStartLine < parent.sourceStartLine
          || current.sourceEndLine > parent.sourceEndLine) {
        throw new Error(`Publication section escapes its parent source range: ${current.id}`);
      }
      if (Number.isInteger(current.ordinal) && Number.isInteger(parent.ordinal)
          && parent.ordinal >= current.ordinal) {
        throw new Error(`Publication parent does not precede its child: ${current.id}`);
      }
      visited.add(parent.id);
      current = parent;
      depth += 1;
    }
    rootById.set(section.id, current.id);
    depthById.set(section.id, depth);
  }
  const roots = sections.filter((section) => rootById.get(section.id) === section.id);
  if (!roots.some((section) => section.role === 'bodymatter')) {
    throw new Error('Publication map must contain a bodymatter root.');
  }

  const unitIds = new Set();
  const units = sourceMap.translationUnits.map((unit) => {
    requireCanonicalId(unit.id, 'Translation unit ID');
    requireCanonicalId(unit.publicationSectionId, `Publication section ID for ${unit.id}`);
    if (unitIds.has(unit.id)) throw new Error(`Duplicate translation unit ID: ${unit.id}`);
    unitIds.add(unit.id);
    if (!byId.has(unit.publicationSectionId)) {
      throw new Error(`Translation unit has an invalid publication section: ${unit.id || 'missing-id'}`);
    }
    return { ...unit, publicationRootId: rootById.get(unit.publicationSectionId) };
  });
  const unitForLine = (line) => units.find((unit) => (
    Number.isInteger(unit.sourceStartLine)
      && Number.isInteger(unit.sourceEndLine)
      && unit.sourceStartLine <= line
      && line <= unit.sourceEndLine
  ));
  const renderedIds = new Set(sections.map((section) => section.id));
  const notes = (Array.isArray(publicationMap.notes) ? publicationMap.notes : []).map((note) => {
    requireCanonicalId(note.id, 'Publication note ID');
    requireCanonicalId(note.publicationSectionId, `Publication section ID for ${note.id}`);
    if (!byId.has(note.publicationSectionId)
        || !Array.isArray(note.referenceIds)
        || !Array.isArray(note.referenceSourceLines)
        || note.referenceIds.length !== note.referenceSourceLines.length) {
      throw new Error(`Publication note has an invalid source mapping: ${note.id || 'missing-id'}`);
    }
    if (renderedIds.has(note.id)) throw new Error(`Duplicate rendered publication ID: ${note.id}`);
    renderedIds.add(note.id);
    for (const referenceId of note.referenceIds) {
      requireCanonicalId(referenceId, `Reference ID for ${note.id}`);
      if (renderedIds.has(referenceId)) {
        throw new Error(`Duplicate rendered publication ID: ${referenceId}`);
      }
      renderedIds.add(referenceId);
    }
    if (note.backlinkIds !== undefined) {
      if (!Array.isArray(note.backlinkIds)) {
        throw new Error(`Backlink IDs for ${note.id} must be an array.`);
      }
      note.backlinkIds.forEach((backlinkId) => {
        requireCanonicalId(backlinkId, `Backlink ID for ${note.id}`);
      });
    }
    const definitionUnit = Number.isInteger(note.sourceStartLine)
      ? unitForLine(note.sourceStartLine) : null;
    const definitionRootId = definitionUnit?.publicationRootId
      || rootById.get(note.publicationSectionId);
    const referenceRootById = Object.create(null);
    note.referenceIds.forEach((referenceId, index) => {
      const referenceUnit = unitForLine(note.referenceSourceLines[index]);
      referenceRootById[referenceId] = referenceUnit?.publicationRootId || definitionRootId;
    });
    return { ...note, definitionRootId, referenceRootById };
  });
  const compiledSections = sections.map((section) => ({
    ...section,
    depth: depthById.get(section.id),
    publicationRootId: rootById.get(section.id),
  }));
  const documents = roots.map((rootSection) => ({
    id: rootSection.id,
    href: `${rootSection.id}.xhtml`,
    title: rootSection.title,
    shortTitle: rootSection.shortTitle,
    role: rootSection.role,
    kind: rootSection.kind,
    sectionIds: compiledSections
      .filter((section) => section.publicationRootId === rootSection.id)
      .map((section) => section.id),
    translationUnitIds: units
      .filter((unit) => unit.publicationRootId === rootSection.id)
      .map((unit) => unit.id),
  }));
  const hrefByRoot = new Map(documents.map((document) => [document.id, document.href]));
  const navigation = compiledSections.map((section) => ({
    id: section.id,
    parentId: section.parentId || null,
    depth: section.depth,
    rootId: section.publicationRootId,
    href: `${hrefByRoot.get(section.publicationRootId)}#${section.id}`,
    label: section.shortTitle,
  }));
  const landmarks = ['frontmatter', 'bodymatter', 'backmatter']
    .map((role) => roots.find((section) => section.role === role))
    .filter(Boolean)
    .map((section) => ({
      role: section.role,
      sectionId: section.id,
      href: `${hrefByRoot.get(section.id)}#${section.id}`,
    }));
  return {
    schema: 'publication-structure-build-plan-v1',
    sections: compiledSections,
    roots,
    translationUnits: units,
    notes,
    documents,
    navigation,
    landmarks,
  };
}

if (require.main === module) {
  try {
    const rootIndex = process.argv.indexOf('--project-root');
    const projectRoot = rootIndex >= 0 ? process.argv[rootIndex + 1] : process.cwd();
    process.stdout.write(`${JSON.stringify(compilePublicationStructure(projectRoot))}\n`);
  } catch (error) {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  }
}

module.exports = { compilePublicationStructure };
