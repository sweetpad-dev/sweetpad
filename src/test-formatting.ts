// Test file with bad formatting
const badFormatting = { foo: "bar", baz: 123 };
function test() {
  return badFormatting.foo || "default";
}
