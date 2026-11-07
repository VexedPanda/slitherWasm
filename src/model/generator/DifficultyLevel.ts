import { Enumeration, EnumerationValue } from 'scenerystack/phet-core';

/**
 * The puzzle difficulties, by name only. The main thread uses these (settings, and naming the difficulty for the
 * generation worker), so it doesn't load CanSolveDifficulty, whose pattern rule data is many megabytes and is only
 * needed where puzzles are generated (in workers). Each value has a CanSolveDifficulty with the same name.
 */
export default class DifficultyLevel extends EnumerationValue {
  public static readonly EASY = new DifficultyLevel();
  public static readonly MEDIUM = new DifficultyLevel();
  public static readonly HARD = new DifficultyLevel();
  public static readonly VERY_HARD = new DifficultyLevel();
  public static readonly FULL = new DifficultyLevel();
  public static readonly NO_LIMIT = new DifficultyLevel();

  public static readonly enumeration = new Enumeration(DifficultyLevel);
}
