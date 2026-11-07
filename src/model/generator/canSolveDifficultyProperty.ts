import DifficultyLevel from './DifficultyLevel.ts';

import { LocalStorageEnumerationProperty } from '../../util/localStorage.ts';

export const canSolveDifficultyProperty = new LocalStorageEnumerationProperty(
  'canSolveDifficulty',
  DifficultyLevel.EASY,
);
