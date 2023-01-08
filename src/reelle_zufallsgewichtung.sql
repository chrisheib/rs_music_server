select rating,
count(*) count,
count(*) * (case rating when 0 then 0 when 1 then 1 when 2 then 2 when 3 then 4 when 4 then 8 when 5 then 16 when 6 then 32 when 7 then 64 end) summed_weight,
--(select sum((case rating when 0 then 0 when 1 then 1 when 2 then 2 when 3 then 4 when 4 then 8 when 5 then 16 when 6 then 32 when 7 then 64 end)) from songs) gesamt_weight,
printf('%.2f', ((sum((case rating when 0 then 0 when 1 then 1 when 2 then 2 when 3 then 4 when 4 then 8 when 5 then 16 when 6 then 32 when 7 then 64 end))*1.0 /
	(select sum((case rating when 0 then 0 when 1 then 1 when 2 then 2 when 3 then 4 when 4 then 8 when 5 then 16 when 6 then 32 when 7 then 64 end))*1.0 from songs))* 100.0))  chance_perc
from songs
group by rating
order by rating desc

select rating,
count(*) count,
count(*) * (case rating when 0 then 0 when 1 then 1 when 2 then 3 when 3 then 9 when 4 then 27 when 5 then 81 when 6 then 243 when 7 then 729 end) summed_weight,
--(select sum((case rating when 0 then 0 when 1 then 1 when 2 then 3 when 3 then 9 when 4 then 27 when 5 then 81 when 6 then 243 when 7 then 729 end)) from songs) gesamt_weight,
printf('%.2f', ((sum((case rating when 0 then 0 when 1 then 1 when 2 then 3 when 3 then 9 when 4 then 27 when 5 then 81 when 6 then 243 when 7 then 729 end))*1.0 /
	(select sum((case rating when 0 then 0 when 1 then 1 when 2 then 3 when 3 then 9 when 4 then 27 when 5 then 81 when 6 then 243 when 7 then 729 end))*1.0 from songs))* 100.0))  chance_perc
from songs
group by rating
order by rating desc

select rating,
count(*) count,
count(*) * (case rating when 0 then 0 when 1 then 1 when 2 then 2.5 when 3 then 6.3 when 4 then 15.6 when 5 then 39.1 when 6 then 97.7 when 7 then 244.1 end) summed_weight,
--(select sum((case rating when 0 then 0 when 1 then 1 when 2 then 2.5 when 3 then 6.3 when 4 then 15.6 when 5 then 39.1 when 6 then 97.7 when 7 then 244.1 end)) from songs) gesamt_weight,
printf('%.2f', ((sum((case rating when 0 then 0 when 1 then 1 when 2 then 2.5 when 3 then 6.3 when 4 then 15.6 when 5 then 39.1 when 6 then 97.7 when 7 then 244.1 end))*1.0 /
	(select sum((case rating when 0 then 0 when 1 then 1 when 2 then 2.5 when 3 then 6.3 when 4 then 15.6 when 5 then 39.1 when 6 then 97.7 when 7 then 244.1 end))*1.0 from songs))* 100.0))  chance_perc
from songs
group by rating
order by rating desc

1    1   1
3    2   2.5
9    3   6.3
27   4   15.6
81   5   39.1
243  6   97.7
729  7   244.1