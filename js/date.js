const { DateTime } = require("luxon")
var dt0 = DateTime.now().setZone('utc')

console.log('ISO')
console.log(dt0.toISO())

console.log('Day')
console.log(dt0.toFormat('yyyyLLdd'))
var dt1 = DateTime.fromSeconds(Math.floor(dt0.toSeconds())).setZone('utc')
console.log(dt1.minus({ hours: dt1.hour, minutes: dt1.minute, seconds: dt1.second }).toISO())

console.log('Week')
console.log(dt0.toFormat("yyyy'w'WW"))
var dt2 = DateTime.fromSeconds(Math.floor(dt0.toSeconds())).setZone('utc')
dt2 = dt2.minus({ days: dt2.weekday - 1, hours: dt2.hour, minutes: dt2.minute, seconds: dt2.second })
console.log(dt2.toISO() + ' w' + dt2.toFormat('WW'))

console.log('Month')
console.log(dt0.toFormat('yyyyLL'))
var dt3 = DateTime.fromSeconds(Math.floor(dt0.toSeconds())).setZone('utc')
console.log(dt3.minus({ days: dt3.day - 1, hours: dt3.hour, minutes: dt3.minute, seconds: dt3.second }).toISO())

console.log('Quarter')
console.log(dt0.toFormat('yyyy') + 'q' + dt0.quarter)
var dt4 = DateTime.fromObject({year: dt0.year, month: ((dt0.quarter - 1) * 3) + 1, day: 1, hour: 0, minute: 0, second: 0}).setZone('utc')
console.log(dt4.toISO() + ' q' + dt4.quarter)

console.log('Year')
console.log(dt0.toFormat('yyyy'))
var dt5 = DateTime.fromSeconds(Math.floor(dt0.toSeconds())).setZone('utc')
console.log(dt5.minus({ months: dt5.month - 1, days: dt5.day - 1, hours: dt5.hour, minutes: dt5.minute, seconds: dt5.second }).toISO())

