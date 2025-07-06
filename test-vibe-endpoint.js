#!/usr/bin/env node

// Quick manual test script for the createVibeTask endpoint
// Run this with: node test-vibe-endpoint.js

const testCreateVibeTask = async () => {
  const testData = {
    agentType: 'test-agent',
    templateId: 'hello-world-ui',
    repositoryName: 'my-test-website-from-template',
    message: 'Create a test website from the template',
    start: false,
  };

  try {
    console.log('Testing createVibeTask endpoint...');
    console.log('Request data:', JSON.stringify(testData, null, 2));

    const response = await fetch('http://localhost:8000/api/tasks/vibe', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
      },
      body: JSON.stringify(testData),
    });

    console.log('Response status:', response.status);
    
    if (response.ok) {
      const result = await response.json();
      console.log('✅ Success! Response:', JSON.stringify(result, null, 2));
      return result;
    } else {
      const error = await response.text();
      console.log('❌ Error response:', error);
      return null;
    }
  } catch (error) {
    console.log('❌ Network error:', error.message);
    return null;
  }
};

const testInvalidTemplate = async () => {
  const testData = {
    agentType: 'test-agent',
    templateId: 'invalid-template-id',
    repositoryName: 'should-fail',
    start: false,
  };

  try {
    console.log('\nTesting invalid template ID...');
    console.log('Request data:', JSON.stringify(testData, null, 2));

    const response = await fetch('http://localhost:8000/api/tasks/vibe', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
      },
      body: JSON.stringify(testData),
    });

    console.log('Response status:', response.status);
    
    if (response.status === 400) {
      const error = await response.json();
      console.log('✅ Expected error response:', JSON.stringify(error, null, 2));
    } else {
      console.log('❌ Expected 400 error but got:', response.status);
    }
  } catch (error) {
    console.log('❌ Network error:', error.message);
  }
};

const main = async () => {
  console.log('🚀 Testing Vibe Task Creation Endpoint');
  console.log('Make sure the backend server is running on http://localhost:8000\n');

  // Test 1: Valid template
  await testCreateVibeTask();

  // Test 2: Invalid template
  await testInvalidTemplate();

  console.log('\n✨ Test completed!');
};

main().catch(console.error);