import * as path from 'node:path'
import type { ParserPlugin } from '@babel/parser'
import { parse } from '@babel/parser'
import { walk } from 'estree-walker'
import MagicString from 'magic-string'

// biome-ignore lint: Using any for AST nodes is acceptable
type ASTNode = any

/**
 * Configuration options for the component tagger
 */
export interface TaggerOptions {
  /** Custom parser plugins to enable (default: ['jsx', 'typescript']) */
  parserPlugins?: ParserPlugin[]
  /** Custom function to determine if an element should be tagged */
  shouldTag?: (elementName: string) => boolean
}

/**
 * Result of the tagging transformation
 */
export interface TagResult {
  /** Transformed source code */
  code: string
  /** Source map */
  map: unknown
}

/**
 * Webpack loader context interface (minimal required fields)
 */
export interface WebpackLoaderContext {
  /** Async callback function */
  async: () => (error: Error | null, content?: string, sourceMap?: unknown) => void
  /** Path to the resource being loaded */
  resourcePath: string
  /** Root context (project root) */
  rootContext?: string
}

/**
 * Vite plugin options
 */
export interface VitePluginOptions extends TaggerOptions {
  /** Only apply in development mode (default: true) */
  devOnly?: boolean
  /** Include/exclude patterns for files to transform */
  include?: string | RegExp | (string | RegExp)[]
  /** Exclude patterns for files to skip */
  exclude?: string | RegExp | (string | RegExp)[]
}

/**
 * Three.js/Fiber elements that should not be tagged
 * These are 3D scene graph elements that don't benefit from component tagging
 */
export const threeFiberElems = new Set([
  'object3D',
  'audioListener',
  'positionalAudio',
  'mesh',
  'batchedMesh',
  'instancedMesh',
  'scene',
  'sprite',
  'lOD',
  'skinnedMesh',
  'skeleton',
  'bone',
  'lineSegments',
  'lineLoop',
  'points',
  'group',
  'camera',
  'perspectiveCamera',
  'orthographicCamera',
  'cubeCamera',
  'arrayCamera',
  'instancedBufferGeometry',
  'bufferGeometry',
  'boxBufferGeometry',
  'circleBufferGeometry',
  'coneBufferGeometry',
  'cylinderBufferGeometry',
  'dodecahedronBufferGeometry',
  'extrudeBufferGeometry',
  'icosahedronBufferGeometry',
  'latheBufferGeometry',
  'octahedronBufferGeometry',
  'planeBufferGeometry',
  'polyhedronBufferGeometry',
  'ringBufferGeometry',
  'shapeBufferGeometry',
  'sphereBufferGeometry',
  'tetrahedronBufferGeometry',
  'torusBufferGeometry',
  'torusKnotBufferGeometry',
  'tubeBufferGeometry',
  'wireframeGeometry',
  'tetrahedronGeometry',
  'octahedronGeometry',
  'icosahedronGeometry',
  'dodecahedronGeometry',
  'polyhedronGeometry',
  'tubeGeometry',
  'torusKnotGeometry',
  'torusGeometry',
  'sphereGeometry',
  'ringGeometry',
  'planeGeometry',
  'latheGeometry',
  'shapeGeometry',
  'extrudeGeometry',
  'edgesGeometry',
  'coneGeometry',
  'cylinderGeometry',
  'circleGeometry',
  'boxGeometry',
  'capsuleGeometry',
  'material',
  'shadowMaterial',
  'spriteMaterial',
  'rawShaderMaterial',
  'shaderMaterial',
  'pointsMaterial',
  'meshPhysicalMaterial',
  'meshStandardMaterial',
  'meshPhongMaterial',
  'meshToonMaterial',
  'meshNormalMaterial',
  'meshLambertMaterial',
  'meshDepthMaterial',
  'meshDistanceMaterial',
  'meshBasicMaterial',
  'meshMatcapMaterial',
  'lineDashedMaterial',
  'lineBasicMaterial',
  'primitive',
  'light',
  'spotLightShadow',
  'spotLight',
  'pointLight',
  'rectAreaLight',
  'hemisphereLight',
  'directionalLightShadow',
  'directionalLight',
  'ambientLight',
  'lightShadow',
  'ambientLightProbe',
  'hemisphereLightProbe',
  'lightProbe',
  'spotLightHelper',
  'skeletonHelper',
  'pointLightHelper',
  'hemisphereLightHelper',
  'gridHelper',
  'polarGridHelper',
  'directionalLightHelper',
  'cameraHelper',
  'boxHelper',
  'box3Helper',
  'planeHelper',
  'arrowHelper',
  'axesHelper',
  'texture',
  'videoTexture',
  'dataTexture',
  'dataTexture3D',
  'compressedTexture',
  'cubeTexture',
  'canvasTexture',
  'depthTexture',
  'raycaster',
  'vector2',
  'vector3',
  'vector4',
  'euler',
  'matrix3',
  'matrix4',
  'quaternion',
  'bufferAttribute',
  'float16BufferAttribute',
  'float32BufferAttribute',
  'float64BufferAttribute',
  'int8BufferAttribute',
  'int16BufferAttribute',
  'int32BufferAttribute',
  'uint8BufferAttribute',
  'uint16BufferAttribute',
  'uint32BufferAttribute',
  'instancedBufferAttribute',
  'color',
  'fog',
  'fogExp2',
  'shape',
  'colorShiftMaterial',
])

/**
 * Determines if a JSX element should be tagged with component metadata
 *
 * @param elementName - Name of the JSX element
 * @param threeDreiImportedElements - Set of imported elements from @react-three/drei
 * @param threeDreiNamespaces - Set of namespace imports from @react-three/drei
 * @returns true if the element should be tagged, false otherwise
 */
export function shouldTagElement(
  elementName: string,
  threeDreiImportedElements: Set<string>,
  threeDreiNamespaces: Set<string>,
): boolean {
  // Skip Three.js/Fiber elements
  if (threeFiberElems.has(elementName)) {
    return false
  }

  // Skip imported drei elements
  if (threeDreiImportedElements.has(elementName)) {
    return false
  }

  // Skip namespaced drei elements (e.g., drei.OrbitControls)
  if (elementName.includes('.')) {
    const parts = elementName.split('.')
    const namespace = parts[0]
    if (namespace && threeDreiNamespaces.has(namespace)) {
      return false
    }
  }

  return true
}

/**
 * Transform JSX/TSX source code to add component debugging attributes
 *
 * @param source - Source code to transform
 * @param filePath - Absolute path to the file being transformed
 * @param cwd - Current working directory (project root)
 * @param options - Tagger configuration options
 * @returns Transformed code and source map
 */
export function transformComponentTags(
  source: string,
  filePath: string,
  cwd: string,
  options: TaggerOptions = {},
): TagResult {
  const { parserPlugins = ['jsx', 'typescript'] as ParserPlugin[], shouldTag: customShouldTag } =
    options

  try {
    const relativePath = path.relative(cwd, filePath)

    // Parse the source code into an AST
    const parserOptions = {
      sourceType: 'module' as const,
      plugins: parserPlugins,
    }
    const ast = parse(source, parserOptions)

    const magicString = new MagicString(source)
    const threeDreiImportedElements = new Set<string>()
    const threeDreiNamespaces = new Set<string>()

    // First pass: collect drei imports
    walk(ast as ASTNode, {
      // biome-ignore lint/complexity/noExcessiveCognitiveComplexity: AST traversal inherently has high complexity
      enter(node: ASTNode) {
        if (node.type === 'ImportDeclaration') {
          const sourceValue = node.source?.value
          if (typeof sourceValue === 'string' && sourceValue.includes('@react-three/drei')) {
            const specifiers = node.specifiers
            for (const spec of specifiers || []) {
              if (spec.type === 'ImportSpecifier') {
                const local = spec.local
                if (local?.name) {
                  threeDreiImportedElements.add(local.name)
                }
              } else if (spec.type === 'ImportNamespaceSpecifier') {
                const local = spec.local
                if (local?.name) {
                  threeDreiNamespaces.add(local.name)
                }
              }
            }
          }
        }
      },
    })

    // Second pass: tag JSX elements
    walk(ast as ASTNode, {
      enter(node: ASTNode) {
        if (node.type === 'JSXOpeningElement') {
          const jsxNode = node
          let elementName: string

          const name = jsxNode.name
          if (name.type === 'JSXIdentifier') {
            elementName = name.name
          } else if (name.type === 'JSXMemberExpression') {
            const memberExpr = name
            const obj = memberExpr.object
            const prop = memberExpr.property
            elementName = `${obj.name}.${prop.name}`
          } else {
            return
          }

          // Skip React Fragments
          if (elementName === 'Fragment' || elementName === 'React.Fragment') {
            return
          }

          // Get location for component ID
          const loc = jsxNode.loc
          const line = loc?.start?.line ?? 0
          const col = loc?.start?.column ?? 0
          const dataComponentId = `${relativePath}:${line}:${col}`

          // Determine if we should tag this element
          const shouldTag = customShouldTag
            ? customShouldTag(elementName)
            : shouldTagElement(elementName, threeDreiImportedElements, threeDreiNamespaces)

          if (shouldTag) {
            const attributeString = ` data-component-id="${dataComponentId}" data-component-name="${elementName}"`

            const nameNode = jsxNode.name
            const insertPosition = nameNode.end ?? 0
            magicString.appendLeft(insertPosition, attributeString)
          }
        }
      },
    })

    const result = magicString.toString()
    const map = magicString.generateMap({ hires: true })

    return { code: result, map }
  } catch (error) {
    // biome-ignore lint/suspicious/noConsole: Error logging is acceptable for debugging transformation failures
    console.error(`Error processing file ${filePath}:`, error)
    // Return original source on error
    return { code: source, map: null }
  }
}
