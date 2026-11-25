// Upgrades FBX 6.x binary files (FBX 2006–2009; what 3ds Max writes with the
// old "FBX 6.1" preset) to FBX 7.x ASCII text that three.js's FBXLoader can
// read. The loader rejects binary files below version 6400 and ASCII files
// below 7000, but a 6.x binary has the same node layout as 7.x; only the
// object model differs:
//   - geometry lives inside each Model instead of in a separate Geometry object
//   - objects are linked by name ("Model::x") instead of by numeric id
//   - textures connect to models, and a per-polygon LayerElementTexture says
//     which texture goes with which polygon, instead of connecting to materials
//   - properties are Properties60 / Property instead of Properties70 / P
// Meshes, materials and textures are carried over; animation, skinning and
// blend shapes are not.
import { unzlibSync } from 'three/addons/libs/fflate.module.js';

const MAGIC = 'Kaydara FBX Binary  \0';
const SEPARATOR = '\0\x01';            // "name\0\x01Class" inside 6.x object names
const ROOT = 'Model::Scene';           // the 6.x scene root; id 0 in 7.x

// Children of a Model that describe its mesh and move to the Geometry object.
const ARRAY_NODES = new Set([
  'Vertices', 'PolygonVertexIndex', 'Edges', 'Normals', 'NormalsIndex', 'NormalsW',
  'Binormals', 'BinormalsIndex', 'Tangents', 'TangentsIndex', 'UV', 'UVIndex',
  'Colors', 'ColorIndex', 'Materials', 'Smoothing', 'Visibility', 'TextureId',
  'PolygonGroup',
]);
const GEOMETRY_NODES = new Set(['Vertices', 'PolygonVertexIndex', 'Edges', 'GeometryVersion']);
// 6.x texture layer elements → the material property the texture feeds in 7.x.
const TEXTURE_LAYERS = {
  LayerElementTexture: 'DiffuseColor',
  LayerElementDiffuseTextures: 'DiffuseColor',
  LayerElementEmissiveTextures: 'EmissiveColor',
  LayerElementSpecularTextures: 'SpecularColor',
  LayerElementBumpTextures: 'Bump',
  LayerElementNormalMapTextures: 'NormalMap',
  LayerElementTransparentTextures: 'TransparentColor',
};
// 6.x property types the 7.x text parser does not know.
const PROPERTY_TYPES = { Vector: 'Vector3D' };

/** FBX version of a binary buffer, or 0 if it is not a binary FBX. */
export function fbxBinaryVersion(buffer) {
  if (buffer.byteLength < 27) return 0;
  const head = new TextDecoder('latin1').decode(new Uint8Array(buffer, 0, MAGIC.length));
  return head === MAGIC ? new DataView(buffer).getUint32(23, true) : 0;
}

/**
 * The buffer to hand to FBXLoader: the input itself for 7.x files (and for
 * anything that is not a binary FBX), or an FBX 7 ASCII rewrite of a 6.x binary.
 * @param {ArrayBuffer} buffer
 * @returns {ArrayBuffer}
 */
export function upgradeLegacyFbx(buffer) {
  const version = fbxBinaryVersion(buffer);
  if (version === 0 || version >= 7000) return buffer;
  const text = toFbx7(readTree(buffer), version);
  return new TextEncoder().encode(text).buffer;
}

// ---------- binary reader ----------

function readTree(buffer) {
  const view = new DataView(buffer);
  const bytes = new Uint8Array(buffer);
  const utf8 = new TextDecoder();
  let pos = 27;

  const readString = n => {
    const s = utf8.decode(bytes.subarray(pos, pos + n));
    pos += n;
    return s;
  };

  const readArray = type => {
    const count = view.getUint32(pos, true);
    const encoding = view.getUint32(pos + 4, true);
    const length = view.getUint32(pos + 8, true);
    pos += 12;
    let raw = bytes.subarray(pos, pos + length);
    pos += length;
    if (encoding) raw = unzlibSync(raw);
    const data = raw.buffer.slice(raw.byteOffset, raw.byteOffset + raw.byteLength);
    switch (type) {
      case 'f': return Array.from(new Float32Array(data, 0, count));
      case 'd': return Array.from(new Float64Array(data, 0, count));
      case 'i': return Array.from(new Int32Array(data, 0, count));
      case 'l': return Array.from(new BigInt64Array(data, 0, count), Number);
      default: return Array.from(new Uint8Array(data, 0, count));
    }
  };

  const readProp = () => {
    const type = String.fromCharCode(bytes[pos++]);
    let v;
    switch (type) {
      case 'Y': v = view.getInt16(pos, true); pos += 2; return v;
      case 'C': v = bytes[pos] !== 0; pos += 1; return v;
      case 'I': v = view.getInt32(pos, true); pos += 4; return v;
      case 'F': v = view.getFloat32(pos, true); pos += 4; return v;
      case 'D': v = view.getFloat64(pos, true); pos += 8; return v;
      case 'L': v = Number(view.getBigInt64(pos, true)); pos += 8; return v;
      case 'S': v = view.getUint32(pos, true); pos += 4; return readString(v);
      case 'R': v = view.getUint32(pos, true); pos += 4; pos += v; return bytes.slice(pos - v, pos);
      case 'f': case 'd': case 'i': case 'l': case 'b': case 'c': return readArray(type);
      default: throw new Error(`FBX 6: unknown property type '${type}' at byte ${pos - 1}`);
    }
  };

  const readNode = () => {
    const end = view.getUint32(pos, true);
    const count = view.getUint32(pos + 4, true);
    const nameLength = bytes[pos + 12];
    pos += 13;
    if (end === 0) return null;
    const name = readString(nameLength);
    const props = [];
    for (let i = 0; i < count; i++) props.push(readProp());
    const children = [];
    while (pos < end) {
      const child = readNode();
      if (!child) break;
      children.push(child);
    }
    pos = end;
    return { name, props, children };
  };

  const children = [];
  while (pos + 13 <= buffer.byteLength) {
    const node = readNode();
    if (!node) break;
    children.push(node);
  }
  return { name: '', props: [], children };
}

// ---------- 7.x text writer ----------

class Writer {
  constructor() { this.lines = []; this.depth = 0; }
  tab() { return '\t'.repeat(this.depth); }
  open(head) { this.lines.push(`${this.tab()}${head} {`); this.depth++; }
  close() { this.depth--; this.lines.push(`${this.tab()}}`); }
  line(text) { this.lines.push(this.tab() + text); }
  prop(name, value) { this.lines.push(`${this.tab()}${name}: ${value}`); }
  array(name, values) {
    this.open(`${name}: *${values.length}`);
    this.line(`a: ${values.map(num).join(',')}`);
    this.close();
  }
  toString() { return this.lines.join('\n') + '\n'; }
}

const child = (node, name) => node.children.find(c => c.name === name);
const children = (node, name) => node.children.filter(c => c.name === name);
const num = v => (typeof v === 'boolean' ? (v ? 1 : 0) : Number.isFinite(v) ? v : 0);
const clean = s => String(s).replace(/[\r\n{}"]/g, ' ');
const quote = s => `"${clean(s)}"`;
// "name\0\x01Class" → "Class::name"; other strings unchanged.
const ref = s => {
  const i = typeof s === 'string' ? s.indexOf(SEPARATOR) : -1;
  return i < 0 ? s : `${s.slice(i + SEPARATOR.length)}::${s.slice(0, i)}`;
};
const value = v => (typeof v === 'string' ? quote(ref(v)) : num(v));
const values = list => list.map(value).join(',');

// FBXLoader tells ASCII from binary by sampling characters at a few fixed
// positions of the text and refusing it when one of them matches the binary
// magic; which characters land there depends on the content, so the comment
// line is padded until the sample passes.
function toFbx7(root, version) {
  const body = writeBody(root, version);
  for (let pad = 0; pad < 64; pad++) {
    const text = `; FBX 7.4.0 project file
; ${'-'.repeat(52 + pad)}

${body}`;
    if (looksLikeAsciiToThree(text)) return text;
  }
  throw new Error('FBX 6: could not produce a header FBXLoader accepts');
}

function looksLikeAsciiToThree(text) {
  const magic = 'Kaydara\FBX\Binary\\';
  let pos = 0;
  for (let i = 0; i < magic.length; i++) {
    if (text[pos] === magic[i]) return false;
    pos += i + 1;
  }
  return true;
}

function writeBody(root, version) {
  const objects = child(root, 'Objects') ?? { children: [] };
  const w = new Writer();
  const ids = new Map([[ROOT, 0]]);
  let nextId = 1000;
  const register = key => { ids.set(key, nextId); return nextId++; };

  w.open('FBXHeaderExtension: ');
  w.prop('FBXHeaderVersion', 1003);
  w.prop('FBXVersion', 7400);
  w.prop('Creator', quote(`Model Viewer, rewritten from an FBX ${version} binary`));
  w.close();
  const settings = child(objects, 'GlobalSettings') ?? child(root, 'GlobalSettings');
  w.open('GlobalSettings: ');
  w.prop('Version', 1000);
  if (settings) writeProperties(w, settings);
  w.close();

  w.open('Objects: ');
  const models = [];
  for (const node of children(objects, 'Model')) {
    const key = ref(node.props[0]);
    const id = register(key);
    const model = { node, key, id, geometryId: null, materials: [], textures: [] };
    models.push(model);
    if (child(node, 'Vertices')) {
      model.geometryId = nextId++;
      writeGeometry(w, node, model.geometryId, key.replace(/^Model::/, 'Geometry::'));
    }
    writeObject(w, 'Model', id, key, node.props[1] ?? 'Null', node);
  }
  for (const type of ['Material', 'Texture', 'Video']) {
    for (const node of children(objects, type)) {
      const key = ref(node.props[0]);
      writeObject(w, type, register(key), key, node.props[1] ?? '', node);
    }
  }
  w.close();

  // Connections: 6.x links child → parent by name. Material → Model and
  // Video → Texture carry over as they are; Texture → Model is replaced by
  // Texture → Material links worked out from each mesh's texture layer.
  w.open('Connections: ');
  const byId = new Map(models.map(m => [m.id, m]));
  for (const node of children(child(root, 'Connections') ?? { children: [] }, 'Connect')) {
    if (node.props[0] !== 'OO') continue;
    const from = ref(node.props[1]), to = ref(node.props[2]);
    if (!ids.has(from) || !ids.has(to)) continue;
    const fromId = ids.get(from), toId = ids.get(to);
    const model = byId.get(toId);
    if (model && from.startsWith('Material::')) model.materials.push(fromId);
    if (model && from.startsWith('Texture::')) { model.textures.push(fromId); continue; }
    w.prop('C', `"OO",${fromId},${toId}`);
  }
  const linked = new Set();
  for (const model of models) {
    if (model.geometryId !== null) w.prop('C', `"OO",${model.geometryId},${model.id}`);
    for (const [textureId, materialId, property] of textureLinks(model)) {
      const key = `${textureId}/${materialId}/${property}`;
      if (linked.has(key)) continue;
      linked.add(key);
      w.prop('C', `"OP",${textureId},${materialId}, "${property}"`);
    }
  }
  w.close();
  return w.toString();
}

function writeProperties(w, node) {
  const block = child(node, 'Properties60');
  if (!block) return;
  w.open('Properties70: ');
  for (const p of children(block, 'Property')) {
    const [name, type, flags, ...rest] = p.props;
    if (typeof name !== 'string' || typeof type !== 'string' || rest.length === 0) continue;
    if (rest.some(v => typeof v === 'object')) continue;
    const type7 = PROPERTY_TYPES[type] ?? type;
    w.prop('P', `${quote(name)}, ${quote(type7)}, "", ${quote(flags ?? '')},${values(rest)}`);
  }
  w.close();
}

// A Model, Material, Texture or Video: scalar children as they are,
// Properties60 as Properties70, raw Video content as base64, mesh data skipped
// (it goes to the Geometry object).
function writeObject(w, type, id, key, attrType, node) {
  w.open(`${type}: ${id}, ${quote(key)}, ${quote(attrType)}`);
  for (const c of node.children) {
    if (c.name === 'Properties60') { writeProperties(w, node); continue; }
    if (c.name === 'NodeAttributeName' || isMeshNode(c) || c.children.length) continue;
    if (c.name === 'Content' && c.props[0] instanceof Uint8Array) {
      if (c.props[0].length) { w.prop('Content', ','); w.line(quote(base64(c.props[0]))); }
      continue;
    }
    if (c.props.length === 0 || c.props.some(v => typeof v === 'object')) continue;
    w.prop(c.name, values(c.props));
  }
  w.close();
}

function writeGeometry(w, node, id, key) {
  w.open(`Geometry: ${id}, ${quote(key)}, "Mesh"`);
  const layerCounts = {};
  for (const c of node.children) {
    if (ARRAY_NODES.has(c.name)) { w.array(c.name, c.props); continue; }
    if (GEOMETRY_NODES.has(c.name)) { w.prop(c.name, values(c.props)); continue; }
    if (!c.name.startsWith('LayerElement') || c.name in TEXTURE_LAYERS || /Textures$/.test(c.name)) continue;
    const index = layerCounts[c.name] ?? 0;
    layerCounts[c.name] = index + 1;
    w.open(`${c.name}: ${index}`);
    for (const e of c.children) {
      if (ARRAY_NODES.has(e.name)) w.array(e.name, e.props);
      else if (e.props.length && e.children.length === 0) w.prop(e.name, values(e.props));
    }
    w.close();
  }
  w.close();
}

const isMeshNode = c => ARRAY_NODES.has(c.name) || GEOMETRY_NODES.has(c.name)
  || c.name === 'Layer' || c.name.startsWith('LayerElement');

// Per-polygon indices of a layer element: ByPolygon/IndexToDirect reads the
// index array, AllSame repeats its first entry, anything else counts as 0.
function polygonIndices(layer, arrayName, polygons) {
  const mapping = child(layer, 'MappingInformationType')?.props[0];
  const array = child(layer, arrayName)?.props ?? [];
  const out = new Array(polygons);
  for (let p = 0; p < polygons; p++) {
    const v = mapping === 'ByPolygon' ? array[p] : array[0];
    out[p] = typeof v === 'number' ? v : 0;
  }
  return out;
}

// [textureId, materialId, property] triples for a 6.x mesh. Each texture layer
// maps polygons to the model's connected textures; LayerElementMaterial maps
// them to its connected materials; a polygon that uses both links the two.
// A model with textures but no texture layer gets its first texture on every
// material.
function textureLinks(model) {
  const { node, materials, textures } = model;
  const links = [];
  if (materials.length === 0 || textures.length === 0) return links;
  const polygons = (child(node, 'PolygonVertexIndex')?.props ?? []).filter(i => i < 0).length;
  const materialLayer = child(node, 'LayerElementMaterial');
  const materialOf = materialLayer ? polygonIndices(materialLayer, 'Materials', polygons) : new Array(polygons).fill(0);
  let found = false;
  for (const layer of node.children) {
    const property = TEXTURE_LAYERS[layer.name];
    if (!property) continue;
    found = true;
    const textureOf = polygonIndices(layer, 'TextureId', polygons);
    const seen = new Set();
    for (let p = 0; p < polygons; p++) {
      const materialId = materials[materialOf[p]], textureId = textures[textureOf[p]];
      if (materialId === undefined || textureId === undefined || seen.has(materialId)) continue;
      seen.add(materialId);
      links.push([textureId, materialId, property]);
    }
  }
  if (!found) for (const materialId of materials) links.push([textures[0], materialId, 'DiffuseColor']);
  return links;
}

function base64(bytes) {
  let binary = '';
  for (let i = 0; i < bytes.length; i += 0x8000) {
    binary += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
  }
  return btoa(binary);
}
