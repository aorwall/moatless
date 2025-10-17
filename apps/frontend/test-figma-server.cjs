#!/usr/bin/env node

/**
 * Simple HTTP server for testing Figma clipboard functionality
 * Serves the test page and allows fetching the test fixture without CORS issues
 */

const http = require('node:http')
const fs = require('node:fs')
const path = require('node:path')

const PORT = 8765
const HOST = 'localhost'

// MIME types
const mimeTypes = {
	'.html': 'text/html',
	'.js': 'text/javascript',
	'.css': 'text/css',
	'.json': 'application/json',
	'.png': 'image/png',
	'.jpg': 'image/jpg',
	'.gif': 'image/gif',
	'.svg': 'image/svg+xml',
	'.ico': 'image/x-icon',
}

const server = http.createServer((req, res) => {
	console.log(`${req.method} ${req.url}`)

	// Parse the URL
	let filePath = `.${req.url}`
	if (filePath === './') {
		filePath = './test-figma-paste.html'
	}

	// Handle backend fixture path
	if (req.url.includes('/backend/src/services/__tests__/fixtures/')) {
		filePath = `..${req.url}`
	}

	// Get the file extension
	const extname = String(path.extname(filePath)).toLowerCase()
	const contentType = mimeTypes[extname] || 'application/octet-stream'

	// Read and serve the file
	fs.readFile(filePath, (error, content) => {
		if (error) {
			if (error.code === 'ENOENT') {
				console.error(`❌ File not found: ${filePath}`)
				res.writeHead(404, { 'Content-Type': 'text/html' })
				res.end(
					'<h1>404 Not Found</h1><p>The requested file was not found.</p>',
					'utf-8'
				)
			} else {
				console.error(`❌ Server error: ${error.code}`)
				res.writeHead(500)
				res.end(`Server Error: ${error.code}`)
			}
		} else {
			// Success
			res.writeHead(200, {
				'Content-Type': contentType,
				'Access-Control-Allow-Origin': '*',
			})
			res.end(content, 'utf-8')
		}
	})
})

server.listen(PORT, HOST, () => {
	console.log('🎨 Figma Clipboard Test Server')
	console.log('================================')
	console.log('')
	console.log(`✅ Server running at http://${HOST}:${PORT}/`)
	console.log('')
	console.log('📋 Instructions:')
	console.log(`   1. Open http://${HOST}:${PORT}/ in your browser`)
	console.log('   2. Click "Copy Figma Test Data" button')
	console.log('   3. Go to your app (http://localhost:3000)')
	console.log('   4. Paste (Cmd+V) in textarea with Figma paste handler')
	console.log('')
	console.log('Or paste in the test page textarea to verify clipboard data')
	console.log('')
	console.log('Press Ctrl+C to stop the server')
})

// Handle graceful shutdown
process.on('SIGINT', () => {
	console.log('\n\n👋 Shutting down server...')
	server.close(() => {
		console.log('✅ Server closed')
		process.exit(0)
	})
})
