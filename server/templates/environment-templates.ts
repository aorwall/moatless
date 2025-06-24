import type { Environment } from '../../shared/types/environment.js'

export interface EnvironmentTemplate {
  id: string
  name: string
  description: string
  defaultMetadata: Partial<Environment> & { [key: string]: any }
  requiredFiles?: string[]
  setupInstructions?: string
}

export const ENVIRONMENT_TEMPLATES: Record<string, EnvironmentTemplate> = {
  python: {
    id: 'python',
    name: 'Python Project',
    description: 'Standard Python project with pip or poetry',
    defaultMetadata: {
      language: 'python',
      framework: 'none',
      testFramework: 'pytest',
    },
    requiredFiles: ['pyproject.toml', 'requirements.txt', 'setup.py'],
    setupInstructions: 'Ensure Python 3.8+ is installed. Run pip install -r requirements.txt',
  },

  django: {
    id: 'django',
    name: 'Django Web Application',
    description: 'Django web framework project',
    defaultMetadata: {
      language: 'python',
      framework: 'django',
      testFramework: 'pytest',
    },
    requiredFiles: ['manage.py', 'settings.py'],
    setupInstructions: 'Install dependencies and run migrations: python manage.py migrate',
  },

  fastapi: {
    id: 'fastapi',
    name: 'FastAPI Application',
    description: 'Modern Python API with FastAPI',
    defaultMetadata: {
      language: 'python',
      framework: 'fastapi',
      testFramework: 'pytest',
    },
    requiredFiles: ['main.py', 'requirements.txt'],
    setupInstructions: 'Install with pip install -r requirements.txt, run with uvicorn main:app',
  },

  typescript: {
    id: 'typescript',
    name: 'TypeScript Project',
    description: 'Node.js TypeScript project',
    defaultMetadata: {
      language: 'typescript',
      framework: 'none',
      testFramework: 'jest',
    },
    requiredFiles: ['package.json', 'tsconfig.json'],
    setupInstructions: 'Run npm install or yarn install',
  },

  react: {
    id: 'react',
    name: 'React Application',
    description: 'React single-page application',
    defaultMetadata: {
      language: 'typescript',
      framework: 'react',
      testFramework: 'jest',
    },
    requiredFiles: ['package.json', 'src/index.tsx', 'src/App.tsx'],
    setupInstructions: 'Install dependencies with npm install and start with npm start',
  },

  nextjs: {
    id: 'nextjs',
    name: 'Next.js Application',
    description: 'Full-stack React framework',
    defaultMetadata: {
      language: 'typescript',
      framework: 'nextjs',
      testFramework: 'jest',
    },
    requiredFiles: ['package.json', 'next.config.js', 'pages/', 'app/'],
    setupInstructions: 'Run npm install && npm run dev',
  },

  express: {
    id: 'express',
    name: 'Express.js API',
    description: 'Node.js Express API server',
    defaultMetadata: {
      language: 'javascript',
      framework: 'express',
      testFramework: 'jest',
    },
    requiredFiles: ['package.json', 'server.js', 'app.js'],
    setupInstructions: 'Install with npm install, run with npm start',
  },

  java: {
    id: 'java',
    name: 'Java Project',
    description: 'Standard Java project with Maven or Gradle',
    defaultMetadata: {
      language: 'java',
      framework: 'none',
      testFramework: 'junit',
    },
    requiredFiles: ['pom.xml', 'build.gradle'],
    setupInstructions: 'Build with mvn install or gradle build',
  },

  springboot: {
    id: 'springboot',
    name: 'Spring Boot Application',
    description: 'Java Spring Boot web application',
    defaultMetadata: {
      language: 'java',
      framework: 'springboot',
      testFramework: 'junit',
    },
    requiredFiles: ['pom.xml', 'src/main/java/**/Application.java'],
    setupInstructions: 'Run with mvn spring-boot:run',
  },

  go: {
    id: 'go',
    name: 'Go Project',
    description: 'Go language project',
    defaultMetadata: {
      language: 'go',
      framework: 'none',
      testFramework: 'go-test',
    },
    requiredFiles: ['go.mod', 'main.go'],
    setupInstructions: 'Run go mod download && go build',
  },

  rust: {
    id: 'rust',
    name: 'Rust Project',
    description: 'Rust language project',
    defaultMetadata: {
      language: 'rust',
      framework: 'none',
      testFramework: 'cargo-test',
    },
    requiredFiles: ['Cargo.toml', 'src/main.rs'],
    setupInstructions: 'Build with cargo build, run with cargo run',
  },

  ruby: {
    id: 'ruby',
    name: 'Ruby Project',
    description: 'Ruby language project',
    defaultMetadata: {
      language: 'ruby',
      framework: 'none',
      testFramework: 'rspec',
    },
    requiredFiles: ['Gemfile', '.ruby-version'],
    setupInstructions: 'Install dependencies with bundle install',
  },

  rails: {
    id: 'rails',
    name: 'Ruby on Rails Application',
    description: 'Rails web framework project',
    defaultMetadata: {
      language: 'ruby',
      framework: 'rails',
      testFramework: 'rspec',
    },
    requiredFiles: ['Gemfile', 'config/application.rb', 'config/routes.rb'],
    setupInstructions: 'Run bundle install && rails db:create && rails db:migrate',
  },
}

export function getTemplateById(id: string): EnvironmentTemplate | undefined {
  return ENVIRONMENT_TEMPLATES[id]
}

export function detectTemplate(files: string[]): EnvironmentTemplate | undefined {
  // Convert to lowercase for case-insensitive matching
  const lowerFiles = files.map((f) => f.toLowerCase())

  // Check templates in priority order (more specific first)
  const templatePriority = [
    'django',
    'fastapi',
    'rails',
    'springboot',
    'nextjs',
    'react',
    'express',
    'python',
    'typescript',
    'java',
    'go',
    'rust',
    'ruby',
  ]

  for (const templateId of templatePriority) {
    const template = ENVIRONMENT_TEMPLATES[templateId]
    if (!template.requiredFiles) continue

    // Check if any required file pattern matches
    const hasRequiredFile = template.requiredFiles.some((pattern) => {
      // Handle glob patterns
      if (pattern.includes('*')) {
        const regex = new RegExp(pattern.replace(/\*/g, '.*'))
        return files.some((file) => regex.test(file))
      }

      // Handle directory checks
      if (pattern.endsWith('/')) {
        return lowerFiles.some((file) => file.startsWith(pattern.toLowerCase()))
      }

      // Exact file match
      return lowerFiles.includes(pattern.toLowerCase())
    })

    if (hasRequiredFile) {
      return template
    }
  }

  return undefined
}

export function suggestTemplates(language?: string): EnvironmentTemplate[] {
  if (!language) {
    return Object.values(ENVIRONMENT_TEMPLATES)
  }

  return Object.values(ENVIRONMENT_TEMPLATES).filter(
    (template) => template.defaultMetadata.language === language.toLowerCase(),
  )
}
