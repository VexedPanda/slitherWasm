import CanSolveDifficulty from '../CanSolveDifficulty.ts';
import DifficultyLevel from '../DifficultyLevel.ts';
import QUnit from 'qunit';

QUnit.module('DifficultyLevel', () => {
  QUnit.test('matches CanSolveDifficulty', (assert) => {
    // The generation worker looks up the CanSolveDifficulty by the DifficultyLevel's name
    assert.deepEqual(DifficultyLevel.enumeration.keys, CanSolveDifficulty.enumeration.keys);
  });
});
