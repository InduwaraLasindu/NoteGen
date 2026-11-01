import test from 'node:test';
import assert from 'node:assert/strict';
import {
  applyClarificationAnswers,
  averageGlyphGrid,
  buildClarificationQuestions,
  extractNumbers,
  selectConsensus,
  synthesizeGlyphInkGrid,
  validateReaderModel,
  verifyContentRange,
  writerCoverage,
} from './engine';
import {
  buildTrueTypeFont,
  computeTtfTableChecksum,
  generateWriterFontPack,
} from './fontpack';
import { buildWordDocumentXml, buildWordTable } from './docx';
import { INITIAL_MODELS, INITIAL_PAGES, INITIAL_WRITERS } from './demoData';

test('Chunk 1: Consensus deduplicates same-family readers and normalizes trailing punctuation on numbers', () => {
  const numsA = extractNumbers('Revenue is 1250. Growth is 48.5%.');
  const numsB = extractNumbers('Revenue is 1250 Growth is 48.5%');
  assert.deepEqual(numsA, ['1250', '48.5']);
  assert.deepEqual(numsA, numsB);

  const result = selectConsensus(INITIAL_PAGES[0].candidates, 2, 0.92);
  assert.equal(result.independentFamilies, 4);
  assert.equal(result.numericConflict, true); // 1250 vs 1280 in tesseract
  assert.ok(result.agreement > 0.7 && result.agreement < 1.0);
});

test('Chunk 2: Writer profile glyph averaging and 62-char coverage calculation', () => {
  const ada = INITIAL_WRITERS[0];
  const cov = writerCoverage(ada);
  assert.ok(cov.includes('r'));
  assert.ok(cov.includes('A'));
  assert.equal(cov.length, 50);

  const avgR = averageGlyphGrid(ada, 'r');
  assert.equal(avgR.length, 16 * 20);
  assert.ok(avgR.some((v) => v >= 180));
});

test('Chunk 3: TrueType (.ttf) binary generator produces valid SFNT header and 0xB1B0AFBA master checksum', () => {
  const ada = INITIAL_WRITERS[0];
  const pack = generateWriterFontPack(ada, {
    family: 'Ada Lovelace Hand',
    spacing: 90,
    threshold: 120,
  });
  assert.equal(pack.glyphCount, 50);
  assert.ok(pack.ttfBytes.byteLength > 1000);
  // TrueType 0x00010000 version header
  assert.equal(pack.ttfBytes[0], 0x00);
  assert.equal(pack.ttfBytes[1], 0x01);
  assert.equal(pack.ttfBytes[2], 0x00);
  assert.equal(pack.ttfBytes[3], 0x00);
  // Master checksum of entire TTF file must equal 0xB1B0AFBA
  const totalChecksum = computeTtfTableChecksum(pack.ttfBytes);
  assert.equal(totalChecksum, 0xb1b0afba);

  // Verify direct buildTrueTypeFont on single glyph
  const singleTtf = buildTrueTypeFont(
    [
      {
        char: 'r',
        contours: [
          [
            [40, 0],
            [40, 500],
            [220, 500],
            [220, 0],
          ],
        ],
        advance: 320,
      },
    ],
    'TestFont'
  );
  assert.equal(computeTtfTableChecksum(singleTtf), 0xb1b0afba);
});

test('Chunk 4: Clarification questions & digest guard protects edited blocks and applies replacements', () => {
  const qs = buildClarificationQuestions(INITIAL_PAGES, [], 8, 0.9);
  assert.equal(qs.length, 3); // block 2 text, block 3 table cell, block 4 figure meaning

  const { updatedPages, changedCount, contextNotes } = applyClarificationAnswers(
    INITIAL_PAGES,
    qs,
    [
      {
        id: qs[0].id,
        replacement:
          'In steady incompressible flow between Tank A and Tank B, onset of turbulence occurs when the local Reynolds number exceeds 2300 along the constricted throat.',
        context: 'Confirmed handwritten r in turbulence.',
        skip: false,
        learn: true,
        glyph: false,
      },
    ]
  );
  assert.equal(changedCount, 1);
  assert.equal(contextNotes.length, 1);
  assert.equal(updatedPages[0].blocks[1].uncertain, false);

  // Answering again with same digest is skipped by buildClarificationQuestions
  const remainingQs = buildClarificationQuestions(
    updatedPages,
    [
      {
        id: qs[0].id,
        replacement: updatedPages[0].blocks[1].text,
        context: '',
        skip: false,
        learn: false,
        glyph: false,
      },
    ],
    8,
    0.9
  );
  assert.equal(remainingQs.length, 2);
});

test('Chunk 5: 3rd-party model validation blocks non-loopback endpoints and verifies HTTP Range headers', () => {
  const validLocal: typeof INITIAL_MODELS[0] = {
    ...INITIAL_MODELS[0],
    id: 'custom-ollama-qwen',
    endpoint: 'http://127.0.0.1:11434',
    alias: 'qwen2.5-vl:7b',
  };
  assert.doesNotThrow(() => validateReaderModel(validLocal));

  const invalidRemote: typeof INITIAL_MODELS[0] = {
    ...validLocal,
    endpoint: 'https://api.example.com/v1',
  };
  assert.throws(() => validateReaderModel(invalidRemote), /NG203/);

  assert.equal(verifyContentRange('bytes 1024-2047/2048', 1024, 2048), true);
  assert.equal(verifyContentRange('bytes 0-2047/2048', 1024, 2048), false);
});

test('Chunk 6: Word OOXML table & document generator preserves cells, merges, and diagram interpretations', () => {
  const tableXml = buildWordTable(INITIAL_PAGES[0].blocks[2]);
  assert.ok(tableXml.includes('<w:tbl>'));
  assert.ok(tableXml.includes('Inlet Reservoir A'));
  assert.ok(tableXml.includes('1250'));

  const docXml = buildWordDocumentXml(INITIAL_PAGES, {
    keepPageBreaks: true,
    preserveFigures: true,
    reviewThreshold: 0.9,
  });
  assert.ok(docXml.includes('Section 4.2: Siphon &amp; Venturi Flow Balance'));
  assert.ok(docXml.includes('Contextual interpretation:'));
});

test('Chunk 7: GitHub Release v0.3.0 builder compiles NoteGen-Setup-0.3.0.exe & NoteGen-0.3.0.exe (PE32+ x64), .ttf, .docx, manifest, and updated README.md', async () => {
  const fs = await import('node:fs');
  const path = await import('node:path');
  const { buildGitHubReleaseV030 } = await import('./releaseBuilder');
  const manifest = buildGitHubReleaseV030(process.cwd());
  assert.equal(manifest.version, '0.3.0');
  assert.equal(manifest.tag, 'v0.3.0');
  assert.equal(manifest.allChunksPassed, true);
  assert.equal(manifest.assets.length, 14);

  // Verify NoteGen-Setup-0.3.0.exe and NoteGen-0.3.0.exe exist and have valid PE32+ headers
  for (const exeName of ['NoteGen-Setup-0.3.0.exe', 'NoteGen-0.3.0.exe']) {
    const exeAsset = manifest.assets.find((a) => a.name === exeName);
    assert.ok(exeAsset, `Missing ${exeName} in v0.3.0 release manifest`);
    assert.ok(exeAsset.bytes > 10000);
    assert.equal(exeAsset.sha256.length, 64);

    const exeBytes = fs.readFileSync(
      path.join(process.cwd(), 'releases', 'v0.3.0', exeName)
    );
    // 'MZ' DOS signature at 0x00
    assert.equal(exeBytes[0], 0x4d);
    assert.equal(exeBytes[1], 0x5a);
    // 'PE\0\0' signature at 0x80
    assert.equal(exeBytes[0x80], 0x50);
    assert.equal(exeBytes[0x81], 0x45);
    assert.equal(exeBytes[0x82], 0x00);
    assert.equal(exeBytes[0x83], 0x00);
    // Machine = 0x8664 (AMD64) at 0x84..0x85
    assert.equal(exeBytes[0x84], 0x64);
    assert.equal(exeBytes[0x85], 0x86);
  }

  // Verify README.md is updated to NoteGen 0.3.0 and references NoteGen-Setup-0.3.0.exe
  assert.ok(manifest.readmeMarkdown.includes('NoteGen 0.3.0'));
  assert.ok(manifest.readmeMarkdown.includes('NoteGen-Setup-0.3.0.exe'));
  assert.ok(manifest.readmeMarkdown.includes('NoteGen-0.3.0.exe'));
});
