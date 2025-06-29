// This file has intentional formatting issues to test the pre-commit hook
// Adding a new comment to trigger a change
export function testFunction(a: number, b: string) {
  const result = a + b
  return result
}

const obj = {
  name: 'test',
  value: 123,
  nested: {
    key: 'value',
  },
}

function anotherFunction(param1: string, param2: number, param3: boolean) {
  return param1 + param2 + param3
}
