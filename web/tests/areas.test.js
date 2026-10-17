import test from 'node:test';
import assert from 'node:assert/strict';
import { areas, placeholder } from '../src/lib/areas.js';

test('the two route trees are /parent and /watch', () => {
	assert.deepEqual(Object.values(areas).map((a) => a.path).sort(), ['/parent', '/watch']);
});

test('each area renders a placeholder naming itself', () => {
	assert.match(placeholder('parent'), /Parent/);
	assert.match(placeholder('watch'), /Watch/);
});
