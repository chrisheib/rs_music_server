select rating,
count(*) count,
count(*) * (case rating when 0 then 0 when 1 then 1 when 2 then 2 when 3 then 4 when 4 then 8 when 5 then 16 when 6 then 32 when 7 then 64 end) summed_weight,
--(select sum((case rating when 0 then 0 when 1 then 1 when 2 then 2 when 3 then 4 when 4 then 8 when 5 then 16 when 6 then 32 when 7 then 64 end)) from songs) gesamt_weight,
printf('%.2f', ((sum((case rating when 0 then 0 when 1 then 1 when 2 then 2 when 3 then 4 when 4 then 8 when 5 then 16 when 6 then 32 when 7 then 64 end))*1.0 /
	(select sum((case rating when 0 then 0 when 1 then 1 when 2 then 2 when 3 then 4 when 4 then 8 when 5 then 16 when 6 then 32 when 7 then 64 end))*1.0 from songs))* 100.0))  chance_perc
from songs
group by rating
order by rating desc