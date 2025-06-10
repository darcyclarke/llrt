console.log("Hello from LLRT executable!");
console.log("Process arguments:", process.argv);
console.log("Current directory:", process.cwd());
console.log("LLRT version:", process.versions);

// Test some basic functionality
const result = [1, 2, 3].map(x => x * 2);
console.log("Array map test:", result);

// Exit with success
process.exit(0);