// The one Must-never that lives in the browser.
//
// "A missing value is drawn as — and never as 0" is a rule about every number on the
// dashboard, and this file is where it is actually decided for all of them. Until now it
// was enforced by reading. The two halves are tested separately on purpose: that null is
// a dash is the rule everyone remembers, and that zero is *not* a dash is the half that
// makes the rule mean anything — an hour that priced nothing and an hour nobody priced
// have to come out looking different.

import { describe, expect, test } from 'vitest'

import {
  DASH,
  count,
  mean,
  millis,
  money,
  named,
  seconds,
  tokens,
  tools,
  yesNo,
} from './format'

/** Every formatter that takes a nullable number, by the name it is called. Listed rather
 * than discovered, so adding a formatter and forgetting the rule shows up as a formatter
 * that is not in this list rather than as a test that quietly covers one fewer thing. */
const NULLABLE = { money, count, millis, seconds, tokens, mean }

describe('a missing number', () => {
  test.each(Object.entries(NULLABLE))('%s(null) is a dash', (_name, format) => {
    expect(format(null)).toBe(DASH)
  })

  test.each(Object.entries(NULLABLE))('%s(null) is not a zero', (_name, format) => {
    // The rule is not "returns something falsy". `"$0.00"`, `"0"` and `"0 ms"` are all
    // answers a careless null-check would produce and all of them are the failure.
    expect(format(null)).not.toMatch(/0/)
  })
})

describe('a zero', () => {
  test('is a number and says so, in every formatter that takes one', () => {
    expect(money(0)).toBe('$0.00')
    expect(count(0)).toBe('0')
    expect(millis(0)).toBe('0 ms')
    expect(seconds(0)).toBe('0:00')
    expect(tokens(0)).toBe('0')
    expect(mean(0)).toBe('0')
  })
})

describe('the values that are not numbers', () => {
  test('false is an answer and only null is a dash', () => {
    expect(yesNo(null)).toBe(DASH)
    expect(yesNo(false)).toBe('no')
    expect(yesNo(true)).toBe('yes')
  })

  test('a call with no tool calls is a dash, and one with none failed is a bare count', () => {
    expect(tools(null, null)).toBe(DASH)
    expect(tools(0, null)).toBe('0')
    expect(tools(3, 0)).toBe('3')
    expect(tools(3, 1)).toBe('3 · 1 failed')
  })

  test('an unnamed pattern is called what the engine calls it', () => {
    expect(named(4, null)).toBe('#4')
    expect(named(4, '   ')).toBe('#4')
    expect(named(4, 'Transfers')).toBe('Transfers')
  })
})

describe('the shapes the rest of the dashboard reads', () => {
  test('seconds are minutes and seconds, zero-padded', () => {
    expect(seconds(143)).toBe('2:23')
    expect(seconds(9)).toBe('0:09')
  })

  test('thousands of tokens are thousands', () => {
    expect(tokens(999)).toBe('999')
    expect(tokens(1500)).toBe('1.5k')
  })

  test('a mean keeps two decimals at most and no trailing zeroes', () => {
    expect(mean(4)).toBe('4')
    expect(mean(4.5)).toBe('4.5')
    expect(mean(4.567)).toBe('4.57')
  })
})
